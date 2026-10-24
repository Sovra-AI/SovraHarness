//! Pure workspace state machine: authentication, sidebar navigation, Personal AI
//! inspector configuration, AG-UI chat stream, `SovraSRX` 22-specialist roster,
//! and `CogniRouteSLM` + `SovraSRX` routing tree.

use crate::agui::{AgUiWidget, BuildStepState, ChatMessage, default_personal_ai_build_widget};
use crate::chat_history::{ChatHistoryFilter, ChatThread, NEW_CHAT_TITLE};
use crate::memory::Timestamp;
use crate::models::ModelKey;
use crate::protocol::Event;
use crate::providers::ModelTier;
use crate::routing_eval::evaluate_routing;
use crate::routing_tree::{DecisionBackend, RouteEvaluation, RoutingConfig, SelectionMode};
use crate::schedules::{
    ScheduleItem, ScheduleStats, ScheduleStatus, compute_stats, default_schedules,
};
use crate::specialists::{SpecialistAgentSpec, default_sovra_srx_agents};
use crate::status::RuntimeStatus;

/// Top-level screen route (Login vs authenticated Main Workspace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRoute {
    Login,
    MainWorkspace,
}

/// Left sidebar navigation tabs (matching the reference screenshot + `SovraSRX` Schedules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTab {
    Chat,
    MyAis,
    Knowledge,
    Models,
    Schedules,
    Settings,
}

impl NavTab {
    pub const ALL: [Self; 6] = [
        Self::Chat,
        Self::MyAis,
        Self::Knowledge,
        Self::Models,
        Self::Schedules,
        Self::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::MyAis => "My AI's",
            Self::Knowledge => "Knowledge",
            Self::Models => "Models",
            Self::Schedules => "Schedules",
            Self::Settings => "Settings",
        }
    }
}

/// Status badge state for each of the 6 cards in the right "My Personal AI" inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigCardStatus {
    Connected,
    Configured,
    Selected,
    Finalizing,
    Ready,
}

impl ConfigCardStatus {
    pub fn badge_text(&self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Configured => "Configured",
            Self::Selected => "Selected",
            Self::Finalizing => "Finalizing...",
            Self::Ready => "Deployed",
        }
    }

    pub fn is_complete(&self) -> bool {
        !matches!(self, Self::Finalizing)
    }
}

/// One of the 6 configuration rows inside the right "My Personal AI" panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalAiConfigCard {
    pub key: String,
    pub title: String,
    pub subtitle: String,
    pub status: ConfigCardStatus,
}

/// Authenticated user profile shown in the bottom-left workspace switcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserProfile {
    pub initials: String,
    pub full_name: String,
    pub email: String,
    pub workspace_label: String,
}

impl Default for UserProfile {
    fn default() -> Self {
        Self {
            initials: "AA".to_owned(),
            full_name: "Ahmed Al Mansoori".to_owned(),
            email: "ahmed@sovra.ai".to_owned(),
            workspace_label: "Personal Workspace".to_owned(),
        }
    }
}

/// Pure, deterministic state of the entire Sovra desktop UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceState {
    pub runtime_status: RuntimeStatus,
    pub route: ScreenRoute,
    pub active_tab: NavTab,
    pub user: UserProfile,
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    pub threads: Vec<ChatThread>,
    pub history_filter: ChatHistoryFilter,
    pub history_search: String,
    pub history_collapsed: bool,
    pub renaming_thread_id: Option<String>,
    pub queued_prompts: Vec<String>,
    pub is_generating: bool,
    pub ai_name: String,
    pub ai_description: String,
    pub ai_building: bool,
    pub config_cards: Vec<PersonalAiConfigCard>,
    pub schedules: Vec<ScheduleItem>,
    pub specialists: Vec<SpecialistAgentSpec>,
    pub routing: RoutingConfig,
    pub last_route_evaluation: Option<RouteEvaluation>,
    pub search_query: String,
    next_msg_seq: u64,
    next_thread_seq: u64,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        let routing = RoutingConfig::default();
        let specialists = default_sovra_srx_agents();
        let initial_id = "personal-ai-setup".to_owned();
        let initial_thread = ChatThread::new(initial_id.clone(), Timestamp(0));
        Self {
            runtime_status: RuntimeStatus::default(),
            route: ScreenRoute::MainWorkspace,
            active_tab: NavTab::Chat,
            user: UserProfile::default(),
            session_id: initial_id,
            messages: Vec::new(),
            threads: vec![initial_thread],
            history_filter: ChatHistoryFilter::Chats,
            history_search: String::new(),
            history_collapsed: false,
            renaming_thread_id: None,
            queued_prompts: Vec::new(),
            is_generating: false,
            ai_name: "My Personal AI".to_owned(),
            ai_description:
                "A trusted partner that understands my work, thinks like me, and helps me go further."
                    .to_owned(),
            ai_building: false,
            config_cards: default_config_cards(),
            schedules: default_schedules(),
            specialists,
            routing,
            last_route_evaluation: None,
            search_query: String::new(),
            next_msg_seq: 0,
            next_thread_seq: 1,
        }
    }
}

fn default_config_cards() -> Vec<PersonalAiConfigCard> {
    let rows = [
        (
            "knowledge",
            "Knowledge",
            "Docs, notes, emails, links and more.",
            ConfigCardStatus::Connected,
        ),
        (
            "memory",
            "Memory",
            "Remembers what matters to you.",
            ConfigCardStatus::Configured,
        ),
        (
            "behaviour",
            "Behaviour",
            "Tone, style, and working preferences.",
            ConfigCardStatus::Configured,
        ),
        (
            "guardrails",
            "Guardrails",
            "Boundaries, safety and privacy.",
            ConfigCardStatus::Configured,
        ),
        (
            "model",
            "Model",
            "Tree Router · Laya SystemOne",
            ConfigCardStatus::Selected,
        ),
        (
            "deployment",
            "Deployment",
            "Private, secure, under your control.",
            ConfigCardStatus::Finalizing,
        ),
    ];
    rows.into_iter()
        .map(|(key, title, subtitle, status)| PersonalAiConfigCard {
            key: key.to_owned(),
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            status,
        })
        .collect()
}

impl WorkspaceState {
    pub fn login(&mut self, email: &str) {
        let trimmed = email.trim();
        if !trimmed.is_empty() {
            trimmed.clone_into(&mut self.user.email);
        }
        self.route = ScreenRoute::MainWorkspace;
    }

    pub fn logout(&mut self) {
        self.route = ScreenRoute::Login;
    }

    pub fn select_tab(&mut self, tab: NavTab) {
        self.active_tab = tab;
    }

    pub fn set_history_filter(&mut self, filter: ChatHistoryFilter) {
        self.history_filter = filter;
    }

    pub fn set_history_search(&mut self, query: &str) {
        query.clone_into(&mut self.history_search);
    }

    pub fn toggle_history_collapsed(&mut self) {
        self.history_collapsed = !self.history_collapsed;
    }

    /// Returns the currently active [`ChatThread`], if found.
    pub fn active_thread(&self) -> Option<&ChatThread> {
        self.threads
            .iter()
            .find(|t| t.id == self.session_id && t.is_active())
    }

    fn sync_active_thread(&mut self) {
        self.next_msg_seq += 1;
        #[allow(
            clippy::cast_possible_wrap,
            reason = "sequence counters are small positive integers"
        )]
        let ts = Timestamp(self.next_msg_seq as i64);
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == self.session_id) {
            thread.messages.clone_from(&self.messages);
            let effective_ts = thread.updated_at.max(ts);
            thread.refresh_summary(effective_ts);
        } else {
            let mut thread = ChatThread::new(self.session_id.clone(), ts);
            thread.messages.clone_from(&self.messages);
            thread.refresh_summary(ts);
            self.threads.push(thread);
        }
    }

    /// Starts a fresh chat conversation thread (or focuses an existing empty draft).
    pub fn start_new_chat(&mut self) -> ChatThread {
        self.sync_active_thread();
        self.history_filter = ChatHistoryFilter::Chats;
        self.active_tab = NavTab::Chat;
        self.renaming_thread_id = None;
        if let Some(existing_draft) = self
            .threads
            .iter()
            .find(|t| t.is_active() && t.is_untouched_draft())
        {
            let draft = existing_draft.clone();
            self.session_id.clone_from(&draft.id);
            self.messages.clear();
            self.queued_prompts.clear();
            self.is_generating = false;
            return draft;
        }
        self.next_thread_seq += 1;
        self.next_msg_seq += 1;
        #[allow(
            clippy::cast_possible_wrap,
            reason = "sequence counters are small positive integers"
        )]
        let ts = Timestamp(self.next_msg_seq as i64);
        let new_id = format!("chat-session-{}", self.next_thread_seq);
        let thread = ChatThread::new(new_id.clone(), ts);
        self.threads.push(thread.clone());
        self.session_id = new_id;
        self.messages.clear();
        self.queued_prompts.clear();
        self.is_generating = false;
        thread
    }

    /// Switches the active conversation to `session_id`.
    pub fn select_thread(&mut self, session_id: &str) -> bool {
        if self.session_id == session_id {
            self.active_tab = NavTab::Chat;
            return true;
        }
        self.sync_active_thread();
        let Some(target) = self
            .threads
            .iter()
            .find(|t| t.id == session_id && t.is_active())
            .cloned()
        else {
            return false;
        };
        self.session_id.clone_from(&target.id);
        self.messages.clone_from(&target.messages);
        self.queued_prompts.clear();
        self.is_generating = false;
        self.renaming_thread_id = None;
        self.active_tab = NavTab::Chat;
        true
    }

    /// Toggles the pinned status of a conversation thread and returns the updated thread.
    pub fn toggle_thread_pin(&mut self, session_id: &str) -> Option<ChatThread> {
        let thread = self
            .threads
            .iter_mut()
            .find(|t| t.id == session_id && t.is_active())?;
        thread.pinned = !thread.pinned;
        Some(thread.clone())
    }

    /// Begins inline renaming for `session_id`, returning its current title.
    pub fn begin_rename_thread(&mut self, session_id: &str) -> Option<String> {
        let thread = self
            .threads
            .iter()
            .find(|t| t.id == session_id && t.is_active())?;
        self.renaming_thread_id = Some(session_id.to_owned());
        Some(thread.title.clone())
    }

    /// Cancels inline renaming without modifying any thread title.
    pub fn cancel_rename_thread(&mut self) {
        self.renaming_thread_id = None;
    }

    /// Renames `session_id` to `new_title` (ignoring blank titles) and returns the updated thread.
    pub fn rename_thread(&mut self, session_id: &str, new_title: &str) -> Option<ChatThread> {
        self.renaming_thread_id = None;
        let trimmed = new_title.trim();
        let thread = self
            .threads
            .iter_mut()
            .find(|t| t.id == session_id && t.is_active())?;
        if trimmed.is_empty() {
            NEW_CHAT_TITLE.clone_into(&mut thread.title);
        } else {
            trimmed.clone_into(&mut thread.title);
        }
        Some(thread.clone())
    }

    /// Soft-deletes `session_id`. If it was the active session, switches to the most recent remaining thread or creates a fresh draft.
    pub fn delete_thread(&mut self, session_id: &str) -> bool {
        self.next_msg_seq += 1;
        #[allow(
            clippy::cast_possible_wrap,
            reason = "sequence counters are small positive integers"
        )]
        let deleted_at = Timestamp(self.next_msg_seq as i64);
        let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| t.id == session_id && t.is_active())
        else {
            return false;
        };
        thread.deleted_at = Some(deleted_at);
        if self.renaming_thread_id.as_deref() == Some(session_id) {
            self.renaming_thread_id = None;
        }
        if self.session_id == session_id {
            let next_active = self
                .threads
                .iter()
                .filter(|t| t.is_active())
                .max_by_key(|t| t.updated_at)
                .cloned();
            if let Some(next) = next_active {
                self.session_id = next.id;
                self.messages = next.messages;
                self.queued_prompts.clear();
                self.is_generating = false;
            } else {
                self.next_thread_seq += 1;
                let new_id = format!("chat-session-{}", self.next_thread_seq);
                let fresh = ChatThread::new(new_id.clone(), deleted_at);
                self.threads.push(fresh);
                self.session_id = new_id;
                self.messages.clear();
                self.queued_prompts.clear();
                self.is_generating = false;
            }
        }
        true
    }

    /// Creates a new automation schedule and appends it to `self.schedules`.
    pub fn create_schedule(
        &mut self,
        name: &str,
        cadence: &str,
        prompt: &str,
    ) -> Option<ScheduleItem> {
        let name = name.trim();
        let prompt = prompt.trim();
        if name.is_empty() || prompt.is_empty() {
            return None;
        }
        let cadence = if cadence.trim().is_empty() {
            "Daily · 09:00 GST"
        } else {
            cadence.trim()
        };
        let slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let id = format!(
            "sched-{}-{}",
            slug.trim_matches('-'),
            self.schedules.len() + 1
        );
        let item = ScheduleItem {
            id,
            name: name.to_owned(),
            cadence: cadence.to_owned(),
            next_run: "Scheduled".to_owned(),
            last_result: "Ready · Never run".to_owned(),
            prompt: prompt.to_owned(),
            status: ScheduleStatus::Active,
            runs_today: 0,
        };
        self.schedules.push(item.clone());
        Some(item)
    }

    /// Deletes a schedule by `schedule_id`.
    pub fn delete_schedule(&mut self, schedule_id: &str) -> bool {
        let before = self.schedules.len();
        self.schedules.retain(|s| s.id != schedule_id);
        self.schedules.len() < before
    }

    pub fn set_routing_mode(&mut self, mode: SelectionMode) {
        self.routing.mode = mode;
        self.sync_model_card_subtitle();
    }

    pub fn set_decision_backend(&mut self, backend: DecisionBackend) {
        self.routing.backend = backend;
        self.sync_model_card_subtitle();
    }

    pub fn set_specialist_model(
        &mut self,
        specialist_id: &str,
        model_override: Option<ModelKey>,
        preferred_tier: ModelTier,
    ) {
        if let Some(agent) = self.specialists.iter_mut().find(|a| a.id == specialist_id) {
            agent.model_override = model_override;
            agent.preferred_tier = preferred_tier;
        }
    }

    pub fn simulate_route_offline(&mut self, prompt: &str) -> RouteEvaluation {
        let eval = evaluate_routing(
            &self.routing,
            &self.specialists,
            prompt,
            None,
            self.routing.backend,
        );
        self.last_route_evaluation = Some(eval.clone());
        eval
    }

    fn sync_model_card_subtitle(&mut self) {
        let subtitle = match self.routing.mode {
            SelectionMode::SingleGlobalModel => self.routing.global_model.as_ref().map_or_else(
                || "Single Model · Default Gemini".to_owned(),
                |k| format!("Pinned · {}", k.model_id),
            ),
            SelectionMode::PerAgentModel => {
                format!("Per-Agent · {} Specialists", self.specialists.len())
            }
            SelectionMode::DynamicTreeRouting => {
                format!("Tree Router · {}", self.routing.backend.label())
            }
        };
        if let Some(card) = self.config_cards.iter_mut().find(|c| c.key == "model") {
            card.subtitle = subtitle;
        }
    }

    pub fn submit_prompt(&mut self, raw_prompt: &str) -> Option<String> {
        let prompt = raw_prompt.trim();
        if prompt.is_empty() {
            return None;
        }
        if self.is_generating {
            self.queued_prompts.push(prompt.to_owned());
            return None;
        }
        self.append_user_turn(prompt);
        Some(prompt.to_owned())
    }

    /// Stops the visible turn and discards queued work so Stop cannot trigger another request.
    pub fn interrupt_generation(&mut self) {
        self.is_generating = false;
        self.queued_prompts.clear();
        self.sync_active_thread();
    }

    fn append_user_turn(&mut self, prompt: &str) {
        self.next_msg_seq += 1;
        let user_id = format!("msg-user-{}", self.next_msg_seq);
        self.messages
            .push(ChatMessage::user(user_id, prompt, "Just now"));
        self.is_generating = true;
        self.sync_active_thread();
    }

    pub fn activate_preview_ai(&mut self) {
        self.ai_building = false;
        for card in &mut self.config_cards {
            if card.key == "deployment" {
                card.status = ConfigCardStatus::Ready;
            }
        }
        for msg in &mut self.messages {
            for widget in &mut msg.widgets {
                if let AgUiWidget::BuildProgress {
                    steps,
                    footer_quote,
                    ..
                } = widget
                {
                    for step in steps {
                        step.state = BuildStepState::Completed;
                    }
                    "\"Your Personal AI is live and ready.\"".clone_into(footer_quote);
                }
            }
        }
    }

    pub fn toggle_schedule(&mut self, schedule_id: &str) {
        if let Some(item) = self.schedules.iter_mut().find(|s| s.id == schedule_id) {
            item.status = match item.status {
                ScheduleStatus::Active => ScheduleStatus::Paused,
                ScheduleStatus::Paused => ScheduleStatus::Active,
            };
        }
    }

    pub fn trigger_schedule(&mut self, schedule_id: &str) -> Option<String> {
        let (schedule_name, prompt) = self
            .schedules
            .iter_mut()
            .find(|s| s.id == schedule_id)
            .map(|s| {
                s.runs_today += 1;
                "Running now in Chat...".clone_into(&mut s.last_result);
                (
                    s.name.clone(),
                    format!("[Schedule: {}] {}", s.name, s.prompt),
                )
            })?;
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == self.session_id)
            && thread.messages.is_empty()
        {
            thread.kind = crate::chat_history::ConversationKind::Scheduled;
            thread.schedule_id = Some(schedule_id.to_owned());
            thread.title = schedule_name;
        }
        self.history_filter = ChatHistoryFilter::Scheduled;
        self.active_tab = NavTab::Chat;
        self.submit_prompt(&prompt)
    }

    pub fn schedule_stats(&self) -> ScheduleStats {
        compute_stats(&self.schedules)
    }

    pub fn apply_event(&mut self, event: &Event) -> Option<String> {
        self.runtime_status.apply(event);
        match event {
            Event::ChatHistoryLoaded { threads } => {
                for incoming in threads {
                    if let Some(existing) = self.threads.iter_mut().find(|t| t.id == incoming.id) {
                        if existing.messages.is_empty() && !incoming.messages.is_empty() {
                            *existing = incoming.clone();
                        }
                    } else {
                        self.threads.push(incoming.clone());
                    }
                }
                if let Some(active) = self.threads.iter().find(|t| t.id == self.session_id)
                    && self.messages.is_empty()
                    && !active.messages.is_empty()
                {
                    self.messages.clone_from(&active.messages);
                }
                None
            }
            Event::Models(
                crate::models::ModelEvent::Loaded { preferences, .. }
                | crate::models::ModelEvent::PreferencesChanged(preferences),
            ) => {
                self.routing.global_model.clone_from(&preferences.active);
                if let Some(active) = &preferences.active {
                    if let Some(card) = self.config_cards.iter_mut().find(|c| c.key == "model") {
                        card.subtitle.clone_from(&active.model_id);
                    }
                } else {
                    self.sync_model_card_subtitle();
                }
                None
            }
            Event::RouteEvaluated { evaluation } => {
                self.last_route_evaluation = Some((**evaluation).clone());
                None
            }
            Event::Models(_) | Event::Ready { .. } | Event::Pong | Event::Stopped => None,
            Event::AgentResponseStarted { session_id, .. }
            | Event::AgentChunk { session_id, .. }
            | Event::AgentThinkingChunk { session_id, .. }
            | Event::AgUiSurface { session_id, .. }
            | Event::AgentCompleted { session_id }
            | Event::AgentError { session_id, .. }
                if session_id != &self.session_id =>
            {
                None
            }
            Event::AgentResponseStarted { model, .. } => {
                self.append_or_extend_assistant("");
                if let Some(message) = self.messages.last_mut() {
                    message.model.get_or_insert_with(|| model.clone());
                }
                self.sync_active_thread();
                None
            }
            // Chunks only touch the live transcript. Copying it into the history
            // thread on every token made a long answer quadratic; the thread is
            // synced when the response starts, finishes, fails or is stopped.
            Event::AgentChunk { text, .. } => {
                self.append_or_extend_assistant(text);
                None
            }
            Event::AgentThinkingChunk {
                text, elapsed_ms, ..
            } => {
                self.append_or_extend_assistant("");
                if let Some(message) = self.messages.last_mut() {
                    message.thinking.push_str(text);
                    message.thinking_ms = Some(*elapsed_ms);
                }
                None
            }
            Event::AgUiSurface { widget, .. } => {
                self.attach_agui_widget(widget.clone());
                self.sync_active_thread();
                None
            }
            Event::AgentCompleted { .. } => {
                self.is_generating = false;
                self.sync_active_thread();
                self.dequeue_next_prompt()
            }
            Event::AgentError { message, .. } => {
                self.is_generating = false;
                self.append_or_extend_assistant(&format!("Note: {message}"));
                self.sync_active_thread();
                self.dequeue_next_prompt()
            }
        }
    }

    fn append_or_extend_assistant(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
            && last.timestamp == "Just now"
        {
            last.text.push_str(chunk);
            return;
        }
        self.next_msg_seq += 1;
        let id = format!("msg-assistant-{}", self.next_msg_seq);
        self.messages
            .push(ChatMessage::assistant(id, chunk, "Just now", Vec::new()));
    }

    fn attach_agui_widget(&mut self, widget: AgUiWidget) {
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
        {
            if let AgUiWidget::Components { surface_id, version, .. } = &widget
                && let Some(existing) = last.widgets.iter_mut().find(|existing| {
                    matches!(existing, AgUiWidget::Components { surface_id: id, .. } if id == surface_id)
                })
            {
                if let AgUiWidget::Components { version: previous, .. } = existing
                    && version >= previous
                {
                    *existing = widget;
                }
                return;
            }
            last.widgets.push(widget);
        } else {
            self.next_msg_seq += 1;
            let id = format!("msg-assistant-{}", self.next_msg_seq);
            self.messages.push(ChatMessage::assistant(
                id,
                String::new(),
                "Just now",
                vec![widget],
            ));
        }
    }

    fn dequeue_next_prompt(&mut self) -> Option<String> {
        if self.queued_prompts.is_empty() {
            return None;
        }
        let next = self.queued_prompts.remove(0);
        self.append_user_turn(&next);
        Some(next)
    }

    pub fn reset_build_progress(&mut self) {
        self.ai_building = true;
        if let Some(first_assistant) = self.messages.get_mut(1) {
            first_assistant.widgets = vec![default_personal_ai_build_widget()];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_model_is_pinned_to_each_reply_and_saved_with_history() {
        let mut state = WorkspaceState::default();
        let session_id = state.session_id.clone();
        let first = crate::ResponseModel {
            model_id: "provider/first".into(),
            name: "First model".into(),
            provider: "OpenRouter".into(),
        };
        let second = crate::ResponseModel {
            model_id: "local/second".into(),
            name: "Second model".into(),
            provider: "Local server".into(),
        };
        for model in [&first, &second] {
            state.submit_prompt("Hello");
            state.apply_event(&Event::AgentResponseStarted {
                session_id: "another-chat".into(),
                model: model.clone(),
            });
            assert_eq!(
                state.messages.last().unwrap().role,
                crate::MessageRole::User
            );
            state.apply_event(&Event::AgentResponseStarted {
                session_id: session_id.clone(),
                model: model.clone(),
            });
            state.apply_event(&Event::AgentChunk {
                session_id: session_id.clone(),
                text: "Hello back".into(),
            });
            state.apply_event(&Event::AgentCompleted {
                session_id: session_id.clone(),
            });
        }
        assert_eq!(state.messages[1].model.as_ref(), Some(&first));
        assert_eq!(state.messages[3].model.as_ref(), Some(&second));
        assert_eq!(state.active_thread().unwrap().messages, state.messages);
        state.start_new_chat();
        assert!(state.select_thread(&session_id));
        assert_eq!(state.messages[1].model.as_ref(), Some(&first));
        assert_eq!(state.messages[3].model.as_ref(), Some(&second));
    }

    #[test]
    fn reasoning_folds_into_the_reply_and_history_syncs_once_per_turn() {
        let mut state = WorkspaceState::default();
        let session_id = state.session_id.clone();
        state.submit_prompt("Why?");
        for (text, elapsed_ms) in [("Let me ", 400), ("check.", 1_900)] {
            state.apply_event(&Event::AgentThinkingChunk {
                session_id: session_id.clone(),
                text: text.into(),
                elapsed_ms,
            });
        }
        state.apply_event(&Event::AgentThinkingChunk {
            session_id: "another-chat".into(),
            text: "ignored".into(),
            elapsed_ms: 9_000,
        });
        for chunk in ["Because ", "it is."] {
            state.apply_event(&Event::AgentChunk {
                session_id: session_id.clone(),
                text: chunk.into(),
            });
        }
        let reply = state.messages.last().unwrap();
        assert_eq!(reply.thinking, "Let me check.");
        assert_eq!(reply.thinking_ms, Some(1_900));
        assert_eq!(reply.text, "Because it is.");
        // Streaming must not copy the transcript into history on every token.
        assert_eq!(state.active_thread().unwrap().messages.len(), 1);
        state.apply_event(&Event::AgentCompleted { session_id });
        assert_eq!(state.active_thread().unwrap().messages, state.messages);
    }

    #[test]
    fn stopping_mid_answer_keeps_the_partial_reply_in_history() {
        let mut state = WorkspaceState::default();
        let session_id = state.session_id.clone();
        state.submit_prompt("Long answer please");
        state.apply_event(&Event::AgentChunk {
            session_id,
            text: "Partial".into(),
        });
        state.interrupt_generation();
        assert_eq!(state.active_thread().unwrap().messages, state.messages);
    }

    #[test]
    fn surface_revisions_update_in_place_and_ignore_other_sessions() {
        let mut state = WorkspaceState::default();
        state.submit_prompt("Report");
        let event = |session_id: String, version| Event::AgUiSurface {
            session_id,
            widget: AgUiWidget::Components {
                surface_id: "report".into(),
                version,
                components: vec![crate::UiComponent::Progress {
                    label: "Report".into(),
                    value: 50,
                }],
            },
        };
        state.apply_event(&event("other-session".into(), 1));
        assert_eq!(state.messages.len(), 1);
        state.apply_event(&event(state.session_id.clone(), 1));
        state.apply_event(&event(state.session_id.clone(), 2));
        state.apply_event(&event(state.session_id.clone(), 1));
        assert_eq!(state.messages.len(), 2);
        let widgets = &state.messages[1].widgets;
        assert_eq!(widgets.len(), 1);
        assert!(matches!(
            widgets[0],
            AgUiWidget::Components { version: 2, .. }
        ));
    }

    #[test]
    fn new_workspace_has_no_demo_conversation_or_activity() {
        let state = WorkspaceState::default();
        assert_eq!(state.messages, Vec::<ChatMessage>::new());
        assert_eq!(state.queued_prompts, Vec::<String>::new());
        assert!(!state.is_generating);
        assert!(!state.ai_building);
        assert!(state.last_route_evaluation.is_none());
    }

    #[test]
    fn login_and_logout_switch_routes() {
        let mut state = WorkspaceState::default();
        state.logout();
        assert_eq!(state.route, ScreenRoute::Login);
        state.login("barney@sovra.ai");
        assert_eq!(state.route, ScreenRoute::MainWorkspace);
        assert_eq!(state.user.email, "barney@sovra.ai");
    }

    #[test]
    fn queues_prompts_while_generating_and_flushes_on_complete() {
        let mut state = WorkspaceState::default();
        let first = state.submit_prompt("First prompt");
        assert_eq!(first.as_deref(), Some("First prompt"));
        assert!(state.is_generating);

        let second = state.submit_prompt("Queued second prompt");
        assert!(second.is_none());
        assert_eq!(state.queued_prompts, vec!["Queued second prompt"]);

        let next = state.apply_event(&Event::AgentCompleted {
            session_id: state.session_id.clone(),
        });
        assert_eq!(next.as_deref(), Some("Queued second prompt"));
        assert!(state.is_generating);
        assert_eq!(state.queued_prompts.len(), 0);
    }

    #[test]
    fn preview_ai_completes_deployment_and_checklist() {
        let mut state = WorkspaceState::default();
        state.reset_build_progress();
        assert!(state.ai_building);
        state.activate_preview_ai();
        assert!(!state.ai_building);
        assert!(state.config_cards.iter().all(|c| c.status.is_complete()));
    }
    #[test]
    fn stream_chunks_preserve_exact_markdown_and_unicode() {
        let mut state = WorkspaceState::default();
        state.submit_prompt("Explain this");
        for text in [
            "## He",
            "ading\n\n**",
            "世界",
            "**\n\n```rust\n",
            "let x = 1;\n",
            "```",
        ] {
            state.apply_event(&Event::AgentChunk {
                session_id: state.session_id.clone(),
                text: text.into(),
            });
        }
        assert_eq!(
            state.messages.last().unwrap().text,
            "## Heading\n\n**世界**\n\n```rust\nlet x = 1;\n```"
        );
    }
    #[test]
    fn stopping_clears_queued_turns_and_keeps_partial_reply() {
        let mut state = WorkspaceState::default();
        state.submit_prompt("First");
        state.submit_prompt("Queued");
        state.apply_event(&Event::AgentChunk {
            session_id: state.session_id.clone(),
            text: "Partial reply".into(),
        });
        state.interrupt_generation();
        assert!(!state.is_generating);
        assert_eq!(state.queued_prompts, Vec::<String>::new());
        assert_eq!(state.messages.last().unwrap().text, "Partial reply");
        assert_eq!(
            state.apply_event(&Event::AgentCompleted {
                session_id: state.session_id.clone()
            }),
            None
        );
    }

    #[test]
    fn manages_multiple_chat_threads_and_scheduled_runs() {
        let mut state = WorkspaceState::default();
        let first_id = state.session_id.clone();
        state.submit_prompt("Analyze Nvidia Q3 earnings");
        state.apply_event(&Event::AgentChunk {
            session_id: first_id.clone(),
            text: "Nvidia revenue grew strongly.".into(),
        });
        state.apply_event(&Event::AgentCompleted {
            session_id: first_id.clone(),
        });
        assert_eq!(
            state.active_thread().unwrap().title,
            "Analyze Nvidia Q3 earnings"
        );
        assert_eq!(
            state.active_thread().unwrap().last_message_preview,
            "Nvidia revenue grew strongly."
        );

        let second = state.start_new_chat();
        assert_ne!(second.id, first_id);
        assert_eq!(state.messages, Vec::<ChatMessage>::new());
        state.submit_prompt("Refactor the SurrealDB adapter");
        assert_eq!(
            state.active_thread().unwrap().title,
            "Refactor the SurrealDB adapter"
        );

        assert!(state.select_thread(&first_id));
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[0].text, "Analyze Nvidia Q3 earnings");

        let pinned = state.toggle_thread_pin(&first_id).unwrap();
        assert!(pinned.pinned);

        let renamed = state
            .rename_thread(&first_id, "NVDA Earnings Deep Dive")
            .unwrap();
        assert_eq!(renamed.title, "NVDA Earnings Deep Dive");

        assert!(state.delete_thread(&first_id));
        assert_eq!(state.session_id, second.id);
    }
}
