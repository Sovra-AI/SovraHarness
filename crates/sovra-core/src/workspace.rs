//! Pure workspace state machine: authentication, sidebar navigation, Personal AI
//! inspector configuration, AG-UI chat stream, and `SovraSRX` schedules.

use crate::agui::{
    AgUiWidget, BuildStepState, ChatMessage, default_personal_ai_build_widget,
    seeded_onboarding_messages,
};
use crate::protocol::Event;
use crate::schedules::{
    ScheduleItem, ScheduleStats, ScheduleStatus, compute_stats, default_schedules,
};
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
    pub queued_prompts: Vec<String>,
    pub is_generating: bool,
    pub ai_name: String,
    pub ai_description: String,
    pub ai_building: bool,
    pub config_cards: Vec<PersonalAiConfigCard>,
    pub schedules: Vec<ScheduleItem>,
    pub search_query: String,
    next_msg_seq: u64,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            runtime_status: RuntimeStatus::default(),
            route: ScreenRoute::MainWorkspace,
            active_tab: NavTab::Chat,
            user: UserProfile::default(),
            session_id: "personal-ai-setup".to_owned(),
            messages: seeded_onboarding_messages(),
            queued_prompts: Vec::new(),
            is_generating: false,
            ai_name: "My Personal AI".to_owned(),
            ai_description:
                "A trusted partner that understands my work, thinks like me, and helps me go further."
                    .to_owned(),
            ai_building: true,
            config_cards: default_config_cards(),
            schedules: default_schedules(),
            search_query: String::new(),
            next_msg_seq: 2,
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
            "Default Gemini (environment)",
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

    fn append_user_turn(&mut self, prompt: &str) {
        self.next_msg_seq += 1;
        let user_id = format!("msg-user-{}", self.next_msg_seq);
        self.messages
            .push(ChatMessage::user(user_id, prompt, "Just now"));
        self.is_generating = true;
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
        let prompt = self
            .schedules
            .iter_mut()
            .find(|s| s.id == schedule_id)
            .map(|s| {
                s.runs_today += 1;
                "Running now in Chat...".clone_into(&mut s.last_result);
                format!("[Schedule: {}] {}", s.name, s.prompt)
            })?;
        self.active_tab = NavTab::Chat;
        self.submit_prompt(&prompt)
    }

    pub fn schedule_stats(&self) -> ScheduleStats {
        compute_stats(&self.schedules)
    }

    pub fn apply_event(&mut self, event: &Event) -> Option<String> {
        self.runtime_status.apply(event);
        match event {
            Event::Models(
                crate::models::ModelEvent::Loaded { preferences, .. }
                | crate::models::ModelEvent::PreferencesChanged(preferences),
            ) => {
                if let Some(card) = self.config_cards.iter_mut().find(|c| c.key == "model") {
                    card.subtitle = preferences.active.as_ref().map_or_else(
                        || "Default Gemini (environment)".to_owned(),
                        |key| key.model_id.clone(),
                    );
                }
                None
            }
            Event::Models(_) | Event::Ready { .. } | Event::Pong | Event::Stopped => None,
            Event::AgentChunk { text, .. } => {
                self.append_or_extend_assistant(text);
                None
            }
            Event::AgUiSurface { widget, .. } => {
                self.attach_agui_widget(widget.clone());
                None
            }
            Event::AgentCompleted { .. } => {
                self.is_generating = false;
                self.dequeue_next_prompt()
            }
            Event::AgentError { message, .. } => {
                self.is_generating = false;
                self.append_or_extend_assistant(&format!("Note: {message}"));
                self.dequeue_next_prompt()
            }
        }
    }

    fn append_or_extend_assistant(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
            && last.timestamp == "Just now"
        {
            if !last.text.is_empty() {
                last.text.push(' ');
            }
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
        assert!(state.ai_building);
        state.activate_preview_ai();
        assert!(!state.ai_building);
        assert!(state.config_cards.iter().all(|c| c.status.is_complete()));
    }
}
