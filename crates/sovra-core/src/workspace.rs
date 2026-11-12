//! Pure workspace state machine: authentication, sidebar navigation, Personal AI
//! inspector configuration, AG-UI chat stream, `SovraSRX` 22-specialist roster,
//! and `CogniRouteSLM` + `SovraSRX` routing tree.

mod conversations;
use conversations::ConversationRun;

use crate::agui::{AgUiWidget, BuildStepState, ChatMessage, default_personal_ai_build_widget};
use crate::chat_history::{ChatHistoryFilter, ChatThread, NEW_CHAT_TITLE};
use crate::memory::Timestamp;
use crate::models::ModelKey;
use crate::protocol::Event;
use crate::providers::ModelTier;
use crate::routing_eval::evaluate_routing;
use crate::routing_tree::{DecisionBackend, RouteEvaluation, RoutingConfig, SelectionMode};
use crate::schedules::{
    ScheduleEvent, ScheduleItem, ScheduleRun, ScheduleStats, ScheduleStatus, compute_stats,
    create_in_book, default_schedules, delete_in_book, pause_in_book, resume_in_book,
    run_now_in_book,
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
    Skills,
    /// The user's AI team (`SovraSRX` Team).
    Team,
    /// Everything kept from chats: documents, code, tables, diagrams.
    Artifacts,
    /// Links this app can use. Appended so existing discriminants stay stable.
    Connections,
    /// Family or team computers joined together (`crate::group`).
    Groups,
}

impl NavTab {
    pub const ALL: [Self; 11] = [
        Self::Chat,
        Self::Team,
        Self::Artifacts,
        Self::Groups,
        Self::MyAis,
        Self::Knowledge,
        Self::Skills,
        Self::Models,
        Self::Schedules,
        Self::Connections,
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
            Self::Skills => "Skills",
            Self::Team => "Team",
            Self::Artifacts => "Artifacts",
            Self::Connections => "Connections",
            Self::Groups => "Groups",
        }
    }

    pub fn chrome_key(self) -> crate::models::ChromeKey {
        match self {
            Self::Chat => crate::models::ChromeKey::NavChat,
            Self::MyAis => crate::models::ChromeKey::NavMyAis,
            Self::Knowledge => crate::models::ChromeKey::NavKnowledge,
            Self::Models => crate::models::ChromeKey::NavModels,
            Self::Schedules => crate::models::ChromeKey::NavSchedules,
            Self::Settings => crate::models::ChromeKey::NavSettings,
            Self::Skills => crate::models::ChromeKey::NavSkills,
            Self::Team => crate::models::ChromeKey::NavTeam,
            Self::Artifacts => crate::models::ChromeKey::NavArtifacts,
            Self::Connections => crate::models::ChromeKey::NavConnections,
            Self::Groups => crate::models::ChromeKey::NavGroups,
        }
    }

    pub fn tooltip_key(self) -> crate::models::ChromeKey {
        match self {
            Self::Chat => crate::models::ChromeKey::NavChatTooltip,
            Self::MyAis => crate::models::ChromeKey::NavMyAisTooltip,
            Self::Knowledge => crate::models::ChromeKey::NavKnowledgeTooltip,
            Self::Models => crate::models::ChromeKey::NavModelsTooltip,
            Self::Schedules => crate::models::ChromeKey::NavSchedulesTooltip,
            Self::Settings => crate::models::ChromeKey::NavSettingsTooltip,
            Self::Skills => crate::models::ChromeKey::NavSkillsTooltip,
            Self::Team => crate::models::ChromeKey::NavTeamTooltip,
            Self::Artifacts => crate::models::ChromeKey::NavArtifactsTooltip,
            Self::Connections => crate::models::ChromeKey::NavConnectionsTooltip,
            Self::Groups => crate::models::ChromeKey::NavGroupsTooltip,
        }
    }

    #[must_use]
    pub fn label_in(self, language: &crate::models::UiLanguage) -> &'static str {
        crate::models::chrome(language, self.chrome_key())
    }
}

/// The OS window title: the page first so taskbars, Alt-Tab and tiling window
/// managers stay distinguishable when the product name is truncated.
pub fn window_title(
    route: ScreenRoute,
    tab: NavTab,
    product: &str,
    language: &crate::models::UiLanguage,
) -> String {
    match route {
        ScreenRoute::Login => product.to_owned(),
        ScreenRoute::MainWorkspace => format!("{} — {product}", tab.label_in(language)),
    }
}

/// Interactive 5-step guided system tutorial walking new users through Sovra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourStep {
    /// Step 1 of 5 (`NavTab::Chat`): Sending your first message and using starter chips.
    WelcomeChat,
    /// Step 2 of 5 (`NavTab::MyAis`): How Sovra automatically routes between quick and deep models.
    AutomaticRouting,
    /// Step 3 of 5 (`NavTab::Knowledge`): How bi-temporal memory remembers facts without losing history.
    KnowledgeMemory,
    /// Step 4 of 5 (`NavTab::Schedules`): Putting recurring prompts and daily briefings on autopilot.
    AutomatedSchedules,
    /// Step 5 of 5 (`NavTab::Settings`): Personalizing Always-On Context and response style.
    PersonalizeSettings,
}

impl TourStep {
    pub const ALL: [Self; 5] = [
        Self::WelcomeChat,
        Self::AutomaticRouting,
        Self::KnowledgeMemory,
        Self::AutomatedSchedules,
        Self::PersonalizeSettings,
    ];

    pub fn step_number(self) -> usize {
        match self {
            Self::WelcomeChat => 1,
            Self::AutomaticRouting => 2,
            Self::KnowledgeMemory => 3,
            Self::AutomatedSchedules => 4,
            Self::PersonalizeSettings => 5,
        }
    }

    pub fn target_tab(self) -> NavTab {
        match self {
            Self::WelcomeChat => NavTab::Chat,
            Self::AutomaticRouting => NavTab::MyAis,
            Self::KnowledgeMemory => NavTab::Knowledge,
            Self::AutomatedSchedules => NavTab::Schedules,
            Self::PersonalizeSettings => NavTab::Settings,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::WelcomeChat => "1. Chat Naturally in Your Own Words",
            Self::AutomaticRouting => "2. Explore AI choices",
            Self::KnowledgeMemory => "3. How memory works",
            Self::AutomatedSchedules => "4. Save a task to run again",
            Self::PersonalizeSettings => "5. Your Personal Sticky Note & Tone (Settings)",
        }
    }

    pub fn explanation(self) -> &'static str {
        match self {
            Self::WelcomeChat => {
                "Type a question at the bottom and press Enter. A starter suggestion fills the box for you to review. Answers appear as they arrive; some models also share reasoning."
            }
            Self::AutomaticRouting => {
                "Try a simulation of how Sovra could choose an AI for different tasks. Chat currently uses the AI you selected in Models."
            }
            Self::KnowledgeMemory => {
                "Follow a sample story showing a saved event, a current belief, and an older belief kept as history. This page explains memory; it does not show facts extracted from your chats."
            }
            Self::AutomatedSchedules => {
                "Save a summary or checklist here, then click “Run now” to send it into Chat. Automatic background runs are not available yet."
            }
            Self::PersonalizeSettings => {
                "Write your “Always-On Context” (like a sticky note on Sovra's desk so you never repeat yourself) and pick how Sovra explains things (such as Simple & Friendly or Step-by-Step)."
            }
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::WelcomeChat => Some(Self::AutomaticRouting),
            Self::AutomaticRouting => Some(Self::KnowledgeMemory),
            Self::KnowledgeMemory => Some(Self::AutomatedSchedules),
            Self::AutomatedSchedules => Some(Self::PersonalizeSettings),
            Self::PersonalizeSettings => None,
        }
    }

    pub fn previous(self) -> Option<Self> {
        match self {
            Self::WelcomeChat => None,
            Self::AutomaticRouting => Some(Self::WelcomeChat),
            Self::KnowledgeMemory => Some(Self::AutomaticRouting),
            Self::AutomatedSchedules => Some(Self::KnowledgeMemory),
            Self::PersonalizeSettings => Some(Self::AutomatedSchedules),
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent workspace flags; one enum would mix unrelated states"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceState {
    pub runtime_status: RuntimeStatus,
    pub memory_status: crate::MemoryStatus,
    pub route: ScreenRoute,
    pub active_tab: NavTab,
    pub user: UserProfile,
    /// Sign-in state, folded from [`Event::Auth`](crate::Event::Auth).
    pub auth: crate::auth::AuthState,
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
    pub threads: Vec<ChatThread>,
    pub history_filter: ChatHistoryFilter,
    pub history_search: String,
    pub history_collapsed: bool,
    pub renaming_thread_id: Option<String>,
    pub queued_prompts: Vec<String>,
    pub file_drafts: std::collections::BTreeMap<String, crate::files::AttachmentDraft>,
    pub file_setup: Option<crate::files::FileSetup>,
    pub is_generating: bool,
    pub ai_name: String,
    pub ai_description: String,
    pub ai_building: bool,
    pub config_cards: Vec<PersonalAiConfigCard>,
    pub schedules: Vec<ScheduleItem>,
    pub schedule_runs: Vec<ScheduleRun>,
    /// This computer's group, folded from [`Event::Group`](crate::Event::Group).
    pub groups: crate::group::GroupsView,
    pub specialists: Vec<SpecialistAgentSpec>,
    pub routing: RoutingConfig,
    pub last_route_evaluation: Option<RouteEvaluation>,
    pub search_query: String,
    /// Whether model connections/preferences have been loaded from the runtime.
    pub models_loaded: bool,
    /// Whether an active chat LLM is selected and has required credentials.
    pub has_active_llm: bool,
    /// This computer has no AI of its own but its group can answer (work handoff:
    /// in a group with "Use a free computer's AI" on). Prompts then go to the runtime
    /// instead of waiting for setup; the runtime says so if nobody is free.
    pub group_can_answer: bool,
    /// Current deterministic setup progression stage.
    pub setup_stage: crate::models::SetupStage,
    /// Plain-language hint explaining what is needed to finish LLM setup when `!has_active_llm`.
    pub llm_setup_hint: Option<String>,
    /// Active step of the interactive 5-step system tutorial, when running.
    pub tour_step: Option<TourStep>,
    /// Whether the user has completed or dismissed the guided onboarding tutorial.
    pub onboarding_completed: bool,
    /// Chrome and agent reply language. Defaults to English until preferences load.
    pub language: crate::models::UiLanguage,
    /// Session IDs for which the user explicitly renamed the title (prevents auto-title overwrite).
    user_renamed_threads: std::collections::BTreeSet<String>,
    /// Session IDs that have already requested or received an LLM-generated title.
    titled_threads: std::collections::BTreeSet<String>,
    model_connections: Vec<crate::models::ModelConnection>,
    /// The user's AI team, folded from [`Event::Teams`].
    pub team: crate::teams::TeamRoster,
    /// Saved artifacts, folded from [`Event::Artifacts`].
    pub artifacts: crate::artifacts::ArtifactLibrary,
    /// Personal channel connections (`WhatsApp` Personal companion state, QR pairing, bridged messages).
    pub connections: crate::connections::ConnectionsState,
    /// The Google account, folded from [`Event::Google`].
    pub google: crate::google::GoogleConnection,
    /// The GitHub account, folded from [`Event::GitHub`].
    pub github: crate::github::GitHubConnection,
    /// Terminal commands, approvals and saved rules, folded from [`Event::Terminal`].
    pub terminal: crate::terminal::TerminalState,
    /// The composer sends prompts to the whole team instead of Sovra alone.
    pub team_mode: bool,
    pub development: crate::development::DevelopmentState,
    pub remote_control: crate::remote_control::RemoteState,
    /// AG-UI `subagentRunId` → the member it belongs to, for attributing messages.
    team_subagents: std::collections::BTreeMap<String, crate::agui::Speaker>,
    active_model_problem: Option<String>,
    /// A message sent before setup finished, as `(session_id, prompt)`. It is sent
    /// automatically once a model is ready, so nobody has to type it twice.
    prompt_awaiting_setup: Option<(String, String)>,
    background_conversations: std::collections::BTreeMap<String, ConversationRun>,
    observed_at: Timestamp,
    next_msg_seq: u64,
    session_namespace: u128,
    next_thread_seq: u64,
}

impl WorkspaceState {
    /// Starts a workspace with a fresh, externally supplied namespace. The app
    /// supplies OS randomness so even chats absent from loaded history stay distinct.
    /// Without a supplied clock, dates remain the Unix epoch until `observe_time`.
    pub fn new(session_namespace: u128) -> Self {
        Self::new_at(session_namespace, Timestamp(0))
    }

    /// The caller supplies wall time; the pure reducer never invents dates from IDs.
    pub fn new_at(session_namespace: u128, at: Timestamp) -> Self {
        let routing = RoutingConfig::default();
        let specialists = default_sovra_srx_agents();
        let initial_id = format!("chat-{session_namespace:032x}-0");
        let initial_thread = ChatThread::new(initial_id.clone(), at);
        Self {
            runtime_status: RuntimeStatus::default(),
            memory_status: crate::MemoryStatus::Unavailable,
            route: ScreenRoute::MainWorkspace,
            active_tab: NavTab::Chat,
            user: UserProfile::default(),
            auth: crate::auth::AuthState::default(),
            session_id: initial_id,
            messages: Vec::new(),
            threads: vec![initial_thread],
            history_filter: ChatHistoryFilter::Chats,
            history_search: String::new(),
            history_collapsed: false,
            renaming_thread_id: None,
            queued_prompts: Vec::new(),
            file_drafts: std::collections::BTreeMap::new(),
            file_setup: None,
            is_generating: false,
            ai_name: "My Personal AI".to_owned(),
            ai_description:
                "A trusted partner that understands my work, thinks like me, and helps me go further."
                    .to_owned(),
            ai_building: false,
            config_cards: default_config_cards(),
            schedules: default_schedules(),
            groups: crate::group::GroupsView::default(),
            schedule_runs: Vec::new(),
            specialists,
            routing,
            last_route_evaluation: None,
            search_query: String::new(),
            models_loaded: false,
            has_active_llm: false,
            group_can_answer: false,
            setup_stage: crate::models::SetupStage::Loading,
            llm_setup_hint: None,
            tour_step: None,
            onboarding_completed: false,
            language: crate::models::UiLanguage::default(),
            user_renamed_threads: std::collections::BTreeSet::new(),
            titled_threads: std::collections::BTreeSet::new(),
            model_connections: Vec::new(),
            team: crate::teams::TeamRoster::default(),
            artifacts: crate::artifacts::ArtifactLibrary::default(),
            connections: crate::connections::ConnectionsState::default(),
            google: crate::google::GoogleConnection::default(),
            github: crate::github::GitHubConnection::default(),
            terminal: crate::terminal::TerminalState::default(),
            team_mode: false,
            development: crate::development::DevelopmentState::default(),
            remote_control: crate::remote_control::RemoteState::default(),
            team_subagents: std::collections::BTreeMap::new(),
            active_model_problem: None,
            prompt_awaiting_setup: None,
            background_conversations: std::collections::BTreeMap::new(),
            observed_at: at,
            next_msg_seq: 0,
            session_namespace,
            next_thread_seq: 0,
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
    /// Folds a sign-in event and keeps the route and user profile in sync: a live
    /// session shows the workspace; a reported signed-out state gates it behind
    /// the Login screen. Until the runtime reports ([`crate::auth::AuthPhase::Unknown`])
    /// the route is left alone, so tests without an auth runtime keep working.
    pub fn apply_auth(&mut self, event: &crate::auth::AuthEvent) {
        self.auth.apply(event);
        if let Some(session) = self.auth.session() {
            self.user = UserProfile {
                initials: session.initials(),
                full_name: session.full_name.clone(),
                email: session.email.clone(),
                workspace_label: "Personal Workspace".to_owned(),
            };
            self.route = ScreenRoute::MainWorkspace;
        } else if !matches!(self.auth.phase, crate::auth::AuthPhase::Unknown) {
            self.route = ScreenRoute::Login;
        }
    }

    pub fn logout(&mut self) {
        self.route = ScreenRoute::Login;
    }

    pub fn select_tab(&mut self, tab: NavTab) {
        self.active_tab = tab;
    }

    /// Starts the interactive 5-step system tour at Step 1 (`TourStep::WelcomeChat`)
    /// and switches the workspace to its target tab.
    pub fn start_tour(&mut self) -> TourStep {
        let step = TourStep::WelcomeChat;
        self.tour_step = Some(step);
        self.active_tab = step.target_tab();
        step
    }

    /// Advances to the next step in the interactive system tour, or completes the tour
    /// if currently on the final step (`TourStep::PersonalizeSettings`).
    pub fn next_tour_step(&mut self) -> Option<TourStep> {
        let current = self.tour_step?;
        if let Some(next) = current.next() {
            self.tour_step = Some(next);
            self.active_tab = next.target_tab();
            Some(next)
        } else {
            self.tour_step = None;
            self.onboarding_completed = true;
            self.active_tab = NavTab::Chat;
            None
        }
    }

    /// Moves back one step in the interactive system tour.
    pub fn prev_tour_step(&mut self) -> Option<TourStep> {
        let prev = self.tour_step?.previous()?;
        self.tour_step = Some(prev);
        self.active_tab = prev.target_tab();
        Some(prev)
    }

    /// Ends or skips the interactive system tour and marks onboarding completed.
    pub fn end_tour(&mut self) {
        self.tour_step = None;
        self.onboarding_completed = true;
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

    /// Set the observation time before applying user input or runtime events.
    /// A clock rollback must not reverse recorded conversation order.
    pub fn observe_time(&mut self, at: Timestamp) {
        self.observed_at = self.observed_at.max(at);
    }

    /// Last wall time the workspace was told about. Labels and graphs use this.
    pub fn now(&self) -> Timestamp {
        self.observed_at
    }

    fn sync_active_thread(&mut self) {
        if let Some(thread) = self.threads.iter_mut().find(|t| t.id == self.session_id) {
            if thread.messages != self.messages {
                thread.messages.clone_from(&self.messages);
                thread.refresh_summary(thread.updated_at.max(self.observed_at));
            }
        } else {
            let mut thread = ChatThread::new(self.session_id.clone(), self.observed_at);
            thread.messages.clone_from(&self.messages);
            thread.refresh_summary(self.observed_at);
            self.threads.push(thread);
        }
    }

    /// Starts a fresh chat conversation thread (or focuses an existing empty draft).
    pub fn start_new_chat(&mut self) -> ChatThread {
        self.sync_active_thread();
        self.park_conversation();
        self.history_filter = ChatHistoryFilter::Chats;
        self.active_tab = NavTab::Chat;
        self.renaming_thread_id = None;
        if let Some(existing_draft) = self
            .threads
            .iter()
            .find(|t| t.is_active() && t.is_untouched_draft())
        {
            let draft = existing_draft.clone();
            self.focus_conversation(&draft);
            return draft;
        }
        let ts = self.observed_at;
        let new_id = self.fresh_thread_id();
        let thread = ChatThread::new(new_id, ts);
        self.threads.push(thread.clone());
        self.focus_conversation(&thread);
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
        self.park_conversation();
        self.focus_conversation(&target);
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
            self.user_renamed_threads.insert(session_id.to_owned());
        }
        Some(thread.clone())
    }

    /// Applies an AI-generated title to `session_id` unless the user manually renamed it.
    pub fn apply_generated_title(
        &mut self,
        session_id: &str,
        raw_title: &str,
    ) -> Option<ChatThread> {
        if self.user_renamed_threads.contains(session_id) {
            return None;
        }
        let cleaned = crate::chat_history::sanitize_generated_title(raw_title);
        if cleaned.is_empty() {
            return None;
        }
        self.titled_threads.insert(session_id.to_owned());
        let thread = self
            .threads
            .iter_mut()
            .find(|t| t.id == session_id && t.is_active())?;
        thread.title = cleaned;
        Some(thread.clone())
    }

    /// If `session_id` has completed its first exchange and has not yet been auto-titled
    /// by the model (or manually renamed), returns `(session_id, first_user_prompt, first_assistant_reply)`
    /// and marks it as requested so it only triggers once per conversation.
    pub fn take_pending_title_request(
        &mut self,
        session_id: &str,
    ) -> Option<(String, String, String)> {
        if self.user_renamed_threads.contains(session_id)
            || self.titled_threads.contains(session_id)
        {
            return None;
        }
        let thread = self
            .threads
            .iter()
            .find(|t| t.id == session_id && t.is_active())?;
        if thread.kind != crate::chat_history::ConversationKind::Chat {
            return None;
        }
        let first_user = thread
            .messages
            .iter()
            .find(|m| m.role == crate::agui::MessageRole::User && !m.text.trim().is_empty())?;
        let first_assistant = thread
            .messages
            .iter()
            .find(|m| m.role == crate::agui::MessageRole::Assistant && !m.text.trim().is_empty())?;
        self.titled_threads.insert(session_id.to_owned());
        Some((
            session_id.to_owned(),
            first_user.text.clone(),
            first_assistant.text.clone(),
        ))
    }

    /// Soft-deletes `session_id`. If it was the active session, switches to the most recent remaining thread or creates a fresh draft.
    pub fn delete_thread(&mut self, session_id: &str) -> bool {
        let deleted_at = self.observed_at;
        let Some(thread) = self
            .threads
            .iter_mut()
            .find(|t| t.id == session_id && t.is_active())
        else {
            return false;
        };
        thread.deleted_at = Some(deleted_at.max(thread.updated_at));
        self.background_conversations.remove(session_id);
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
                self.focus_conversation(&next);
            } else {
                let new_id = self.fresh_thread_id();
                let fresh = ChatThread::new(new_id, deleted_at);
                self.threads.push(fresh.clone());
                self.focus_conversation(&fresh);
            }
        }
        true
    }

    fn fresh_thread_id(&mut self) -> String {
        loop {
            self.next_thread_seq = self
                .next_thread_seq
                .checked_add(1)
                .expect("a workspace cannot create 2^64 conversations");
            let id = format!(
                "chat-{:032x}-{}",
                self.session_namespace, self.next_thread_seq
            );
            if !self.threads.iter().any(|thread| thread.id == id) {
                return id;
            }
        }
    }

    /// Creates a new automation schedule and appends it to `self.schedules`.
    pub fn create_schedule(
        &mut self,
        name: &str,
        cadence: &str,
        prompt: &str,
    ) -> Option<ScheduleItem> {
        let mut book = self.schedule_book();
        let item =
            create_in_book(&mut book, name, cadence, prompt, None, self.observed_at, 0).ok()?;
        self.apply_schedule_book(book);
        Some(item)
    }

    /// Deletes a schedule by `schedule_id`.
    pub fn delete_schedule(&mut self, schedule_id: &str) -> bool {
        let mut book = self.schedule_book();
        let removed = delete_in_book(&mut book, schedule_id);
        self.apply_schedule_book(book);
        removed
    }

    fn schedule_book(&self) -> crate::schedules::ScheduleBook {
        crate::schedules::ScheduleBook {
            next_seq: self
                .schedules
                .iter()
                .filter_map(|item| item.id.rsplit('-').next()?.parse().ok())
                .max()
                .unwrap_or(0),
            schedules: self.schedules.clone(),
            runs: self.schedule_runs.clone(),
        }
    }

    fn apply_schedule_book(&mut self, book: crate::schedules::ScheduleBook) {
        self.schedules = book.schedules;
        self.schedule_runs = book.runs;
    }

    fn apply_schedule_event(&mut self, event: &ScheduleEvent) {
        match event {
            ScheduleEvent::Snapshot { schedules, runs } => {
                self.schedules.clone_from(schedules);
                self.schedule_runs.clone_from(runs);
            }
            ScheduleEvent::Fired {
                schedule_id,
                session_id,
                name,
            } => {
                if !self.threads.iter().any(|thread| thread.id == *session_id) {
                    self.threads.push(ChatThread::new_scheduled(
                        session_id.clone(),
                        schedule_id.clone(),
                        name,
                        self.observed_at,
                    ));
                }
            }
            ScheduleEvent::RunRecorded { run } => {
                if let Some(existing) = self.schedule_runs.iter_mut().find(|item| item.id == run.id)
                {
                    *existing = run.clone();
                } else {
                    self.schedule_runs.push(run.clone());
                }
            }
            ScheduleEvent::Error(_) => {}
        }
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

    fn refresh_llm_readiness(&mut self) {
        if !self.models_loaded {
            return;
        }
        let temp = crate::models::ModelsState {
            connections: self.model_connections.clone(),
            preferences: crate::models::ModelPreferences {
                active: self.routing.global_model.clone(),
                onboarding_completed: self.onboarding_completed,
                ..crate::models::ModelPreferences::default()
            },
            loaded: true,
            active_problem: self.active_model_problem.clone(),
            ..crate::models::ModelsState::default()
        };
        self.has_active_llm = temp.has_active_llm();
        self.setup_stage = temp.setup_stage();
        self.llm_setup_hint = temp.setup_status_hint();
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
        if self.models_loaded && !self.has_active_llm && !self.group_can_answer {
            self.append_user_turn(prompt);
            self.is_generating = false;
            self.prompt_awaiting_setup = Some((self.session_id.clone(), prompt.to_owned()));
            let widget = crate::agui::llm_setup_widget_for_stage(
                &self.setup_stage,
                self.llm_setup_hint.as_deref(),
            );
            self.attach_agui_widget(widget);
            self.sync_active_thread();
            return None;
        }
        if self.is_generating {
            self.queued_prompts.push(prompt.to_owned());
            return None;
        }
        self.append_user_turn(prompt);
        Some(prompt.to_owned())
    }

    /// File turns remain in the composer until ready and are never queued without their files.
    pub fn submit_file_prompt(
        &mut self,
        prompt: &str,
        attachments: Vec<crate::files::Attachment>,
    ) -> bool {
        if self.is_generating
            || attachments.is_empty()
            || attachments.len() > crate::files::MAX_ATTACHMENTS
        {
            return false;
        }
        self.append_user_turn(prompt);
        if let Some(message) = self.messages.last_mut() {
            message.attachments = attachments;
        }
        self.sync_active_thread();
        true
    }

    /// Stops the visible turn and discards queued work so Stop cannot trigger another request.
    pub fn interrupt_generation(&mut self) {
        self.is_generating = false;
        self.queued_prompts.clear();
        self.sync_active_thread();
    }

    fn append_user_turn(&mut self, prompt: &str) {
        let user_id = self.fresh_message_id("user");
        let mut message = ChatMessage::user(user_id, prompt, "Just now");
        message.occurred_at = Some(self.observed_at);
        self.messages.push(message);
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
        let mut book = self.schedule_book();
        let paused = book
            .schedules
            .iter()
            .find(|item| item.id == schedule_id)
            .is_some_and(|item| item.status == ScheduleStatus::Active);
        if paused {
            let _ = pause_in_book(&mut book, schedule_id);
        } else {
            let _ = resume_in_book(&mut book, schedule_id, self.observed_at);
        }
        self.apply_schedule_book(book);
    }

    pub fn trigger_schedule(&mut self, schedule_id: &str) -> Option<String> {
        let mut book = self.schedule_book();
        let fire = run_now_in_book(
            &mut book,
            schedule_id,
            Some(self.session_id.clone()),
            self.observed_at,
        )
        .ok()
        .flatten();
        self.apply_schedule_book(book);
        let item = self.schedules.iter().find(|item| item.id == schedule_id)?;
        let schedule_name = item.name.clone();
        let prompt = fire.map_or_else(
            || format!("[Schedule: {}] {}", item.name, item.prompt),
            |fire| fire.prompt,
        );
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
        compute_stats(&self.schedule_book(), self.observed_at)
    }

    fn fold_event(&mut self, event: &Event) -> Option<String> {
        self.runtime_status.apply(event);
        match event {
            Event::Schedule(event) => {
                self.apply_schedule_event(event);
                None
            }
            Event::Group(event) => {
                self.groups.apply((**event).clone());
                None
            }
            Event::RemoteControl(event) => {
                self.remote_control.apply(event);
                None
            }
            Event::Development(event) => {
                self.development.apply(event);
                None
            }
            Event::Auth(auth_event) => {
                self.apply_auth(auth_event);
                None
            }
            Event::Memory(crate::MemoryEvent::Status(status)) => {
                self.memory_status = status.clone();
                None
            }
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
            Event::Models(model_event) => {
                match model_event {
                    crate::models::ModelEvent::Loaded {
                        connections,
                        preferences,
                    } => {
                        self.models_loaded = true;
                        self.onboarding_completed = preferences.onboarding_completed;
                        self.language.clone_from(&preferences.language);
                        self.model_connections.clone_from(connections);
                        self.routing.global_model.clone_from(&preferences.active);
                        self.refresh_llm_readiness();
                        if let Some(active) = &preferences.active {
                            if let Some(card) =
                                self.config_cards.iter_mut().find(|c| c.key == "model")
                            {
                                card.subtitle.clone_from(&active.model_id);
                            }
                        } else {
                            self.sync_model_card_subtitle();
                        }
                    }
                    crate::models::ModelEvent::PreferencesChanged(preferences) => {
                        self.onboarding_completed = preferences.onboarding_completed;
                        self.language.clone_from(&preferences.language);
                        self.routing.global_model.clone_from(&preferences.active);
                        self.refresh_llm_readiness();
                        if let Some(active) = &preferences.active {
                            if let Some(card) =
                                self.config_cards.iter_mut().find(|c| c.key == "model")
                            {
                                card.subtitle.clone_from(&active.model_id);
                            }
                        } else {
                            self.sync_model_card_subtitle();
                        }
                    }
                    crate::models::ModelEvent::ConnectionDeleted { id } => {
                        self.model_connections.retain(|c| c.id != *id);
                        if self
                            .routing
                            .global_model
                            .as_ref()
                            .is_some_and(|k| k.connection_id == *id)
                        {
                            self.routing.global_model = None;
                            self.active_model_problem = None;
                        }
                        self.refresh_llm_readiness();
                        self.sync_model_card_subtitle();
                    }
                    crate::models::ModelEvent::ActiveModelStatus { problem } => {
                        self.active_model_problem.clone_from(problem);
                        self.refresh_llm_readiness();
                    }
                    _ => {}
                }
                self.take_prompt_awaiting_setup()
            }
            Event::RouteEvaluated { evaluation } => {
                self.last_route_evaluation = Some((**evaluation).clone());
                None
            }
            Event::ChatTitleGenerated { session_id, title } => {
                let _ = self.apply_generated_title(session_id, title);
                None
            }
            Event::Memory(crate::MemoryEvent::SourceSaved {
                message, receipt, ..
            }) => {
                if let Some(message) = message {
                    self.link_memory_source(message, &receipt.source_id);
                }
                None
            }
            Event::Memory(
                crate::MemoryEvent::Queued { .. }
                | crate::MemoryEvent::Processed(_)
                | crate::MemoryEvent::Review(_)
                | crate::MemoryEvent::Semantic(_),
            )
            | Event::Skills(_)
            | Event::FileIndex(_)
            | Event::Ready { .. }
            | Event::Pong
            | Event::Stopped
            | Event::FilePrepared(_) => None,
            Event::Teams(team_event) => {
                self.team.apply(team_event);
                None
            }
            Event::Artifacts(artifact_event) => {
                self.artifacts.apply(artifact_event);
                None
            }
            Event::Connections(connection_event) => {
                self.connections.apply(connection_event);
                None
            }
            Event::Google(google_event) => {
                self.google.apply(google_event);
                None
            }
            Event::GitHub(github_event) => {
                self.github.apply(github_event);
                None
            }
            Event::Terminal(terminal_event) => {
                self.terminal.apply(terminal_event);
                None
            }
            Event::Mcp(mcp_event) => {
                crate::mcp::record_tool_result(self, mcp_event);
                None
            }
            Event::AgUi { session_id, event } => self.apply_agui(session_id, event),
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

    /// Folds one AG-UI run event into the conversation it belongs to. Events for
    /// another conversation still finish its run, so its composer is not stuck.
    fn apply_agui(&mut self, session_id: &str, event: &crate::agui::AgUiEvent) -> Option<String> {
        use crate::agui::AgUiEvent;
        if session_id != self.session_id {
            return None;
        }
        match event {
            AgUiEvent::RunStarted { .. } => {
                self.is_generating = true;
                None
            }
            AgUiEvent::ActivitySnapshot {
                message_id,
                activity,
            } => {
                let (widget, author) = match activity {
                    crate::agui::LiveActivity::CodeWriting {
                        language,
                        content,
                        artifact_id,
                    } => (
                        AgUiWidget::CodeWriting {
                            language: language.clone(),
                            content: content.clone(),
                            artifact_id: artifact_id.clone(),
                        },
                        "Code",
                    ),
                    crate::agui::LiveActivity::Team(team) => {
                        (AgUiWidget::TeamActivity(team.clone()), "Team")
                    }
                    crate::agui::LiveActivity::Research(board) => {
                        (AgUiWidget::ResearchBoard(board.clone()), "Research")
                    }
                    crate::agui::LiveActivity::Work(work) => {
                        self.set_work_log(work.clone());
                        return None;
                    }
                };
                if let Some(message) = self.messages.iter_mut().find(|m| m.id == *message_id) {
                    message.widgets = vec![widget];
                } else {
                    let mut message =
                        ChatMessage::assistant(message_id.clone(), "", "Just now", vec![widget]);
                    author.clone_into(&mut message.author);
                    if matches!(activity, crate::LiveActivity::CodeWriting { .. })
                        && self.messages.last().is_some_and(|last| {
                            last.role == crate::MessageRole::Assistant
                                && last.timestamp == "Just now"
                        })
                    {
                        self.messages.insert(self.messages.len() - 1, message);
                    } else {
                        self.messages.push(message);
                    }
                }
                None
            }
            AgUiEvent::SubagentStarted {
                subagent_run_id,
                speaker,
                ..
            } => {
                self.team_subagents
                    .insert(subagent_run_id.clone(), speaker.clone());
                None
            }
            AgUiEvent::TextMessageStart {
                message_id,
                subagent_run_id,
            } => {
                if self.messages.iter().any(|m| m.id == *message_id) {
                    return None;
                }
                let speaker = subagent_run_id
                    .as_ref()
                    .and_then(|id| self.team_subagents.get(id))
                    .cloned();
                let mut message =
                    ChatMessage::assistant(message_id.clone(), "", "Just now", Vec::new());
                if let Some(speaker) = speaker {
                    message.author.clone_from(&speaker.name);
                    message.speaker = Some(speaker);
                }
                self.messages.push(message);
                None
            }
            AgUiEvent::TextMessageContent { message_id, delta } => {
                if let Some(message) = self.messages.iter_mut().rev().find(|m| m.id == *message_id)
                {
                    message.text.push_str(delta);
                }
                None
            }
            AgUiEvent::TextMessageEnd { message_id } => {
                // A member that streamed nothing (a pass) leaves no empty bubble.
                self.messages.retain(|m| {
                    m.id != *message_id || !m.text.trim().is_empty() || !m.widgets.is_empty()
                });
                self.sync_active_thread();
                None
            }
            AgUiEvent::SubagentFinished { .. } => None,
            AgUiEvent::RunFinished { .. } => {
                self.is_generating = false;
                self.team_subagents.clear();
                self.sync_active_thread();
                self.dequeue_next_prompt()
            }
            AgUiEvent::RunError { message } => {
                self.is_generating = false;
                self.team_subagents.clear();
                self.append_or_extend_assistant(&format!("Note: {message}"));
                self.sync_active_thread();
                self.dequeue_next_prompt()
            }
        }
    }

    /// Whether `prompt` goes to the team: team mode, `/team …` or an `@member`.
    /// Returns the prompt to send without the `/team` command.
    pub fn team_prompt<'a>(&self, prompt: &'a str) -> Option<&'a str> {
        if let Some(rest) = crate::teams::strip_team_command(prompt) {
            return Some(rest);
        }
        (self.team_mode || crate::teams::mentions_member(prompt, &self.team.members))
            .then_some(prompt)
    }

    /// Earlier visible conversation; the current request travels separately.
    pub fn team_history(&self) -> Vec<crate::teams::TranscriptLine> {
        let context = crate::ConversationContext::before_current_turn(&self.messages);
        context.team_history()
    }

    /// A reply that came from a connected service. The speaker line names that service.
    pub(crate) fn push_service_reply(
        &mut self,
        service_name: &str,
        text: &str,
        widget: Option<AgUiWidget>,
    ) {
        let id = self.fresh_message_id("assistant");
        let mut message =
            ChatMessage::assistant(id, text, "Just now", widget.into_iter().collect());
        service_name.clone_into(&mut message.author);
        message.speaker = Some(crate::agui::Speaker {
            member_id: format!("service:{service_name}"),
            name: service_name.to_owned(),
            icon: "link".into(),
            role: "Connected service".into(),
        });
        self.messages.push(message);
        self.sync_active_thread();
    }

    /// Message ids key per-message UI state, so they must be unique within the
    /// conversation. The counter restarts every launch while restored messages keep
    /// their old ids, so skip any id that is already taken.
    fn fresh_message_id(&mut self, role: &str) -> String {
        loop {
            self.next_msg_seq += 1;
            let id = format!("msg-{role}-{}", self.next_msg_seq);
            if !self.messages.iter().any(|message| message.id == id) {
                return id;
            }
        }
    }

    fn append_or_extend_assistant(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
            && last.timestamp == "Just now"
            && !last
                .widgets
                .iter()
                .any(|widget| matches!(widget, AgUiWidget::CodeWriting { .. }))
        {
            last.text.push_str(chunk);
            return;
        }
        let id = self.fresh_message_id("assistant");
        self.messages
            .push(ChatMessage::assistant(id, chunk, "Just now", Vec::new()));
    }

    /// The agent loop's live log belongs to the reply it is producing: it sits on
    /// the current assistant message (above its answer) and is replaced in place by
    /// each snapshot, so the answer streams into the same bubble underneath it.
    fn set_work_log(&mut self, work: crate::work::WorkActivity) {
        let widget = AgUiWidget::WorkLog(work);
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
            && last.timestamp == "Just now"
            && last.speaker.is_none()
            && !last
                .widgets
                .iter()
                .any(|widget| matches!(widget, AgUiWidget::CodeWriting { .. }))
        {
            if let Some(existing) = last
                .widgets
                .iter_mut()
                .find(|existing| matches!(existing, AgUiWidget::WorkLog(_)))
            {
                *existing = widget;
            } else {
                last.widgets.insert(0, widget);
            }
            return;
        }
        let id = self.fresh_message_id("assistant");
        self.messages.push(ChatMessage::assistant(
            id,
            String::new(),
            "Just now",
            vec![widget],
        ));
    }

    fn attach_agui_widget(&mut self, widget: AgUiWidget) {
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
            && !last
                .widgets
                .iter()
                .any(|widget| matches!(widget, AgUiWidget::CodeWriting { .. }))
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
            let id = self.fresh_message_id("assistant");
            self.messages.push(ChatMessage::assistant(
                id,
                String::new(),
                "Just now",
                vec![widget],
            ));
        }
    }

    /// Once setup completes, resends the message that was waiting for it in the
    /// same conversation and clears the setup card that asked for it.
    fn take_prompt_awaiting_setup(&mut self) -> Option<String> {
        if !self.has_active_llm || self.is_generating {
            return None;
        }
        let (session_id, prompt) = self.prompt_awaiting_setup.take()?;
        if session_id != self.session_id {
            return None;
        }
        if let Some(last) = self.messages.last_mut()
            && last.role == crate::agui::MessageRole::Assistant
        {
            last.widgets.retain(|widget| {
                !matches!(widget, AgUiWidget::Components { surface_id, .. } | AgUiWidget::SurfaceCard { surface_id, .. }
                    if surface_id == crate::agui::LLM_SETUP_SURFACE_ID)
            });
            if last.widgets.is_empty() && last.text.is_empty() {
                self.messages.pop();
            }
        }
        self.is_generating = true;
        self.sync_active_thread();
        Some(prompt)
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
    fn source_activity_does_not_split_or_absorb_the_streamed_reply() {
        for prefix in ["", "Here is your page:\n```html\n"] {
            let mut state = WorkspaceState::new(0);
            let session = state.session_id.clone();
            if !prefix.is_empty() {
                state.apply_event(&Event::AgentChunk {
                    session_id: session.clone(),
                    text: prefix.into(),
                });
            }
            state.apply_event(&Event::AgUi {
                session_id: session.clone(),
                event: crate::AgUiEvent::ActivitySnapshot {
                    message_id: "source".into(),
                    activity: crate::LiveActivity::CodeWriting {
                        language: "html".into(),
                        content: "<h1>Hello</h1>".into(),
                        artifact_id: None,
                    },
                },
            });
            state.apply_event(&Event::AgentChunk {
                session_id: session,
                text: "<h1>Hello</h1>\n```".into(),
            });
            assert_eq!(
                state
                    .messages
                    .iter()
                    .find(|message| message.id == "source")
                    .unwrap()
                    .text,
                ""
            );
            assert_eq!(
                state.messages.last().unwrap().text,
                format!("{prefix}<h1>Hello</h1>\n```")
            );
        }
    }

    #[test]
    fn a_new_workspace_starts_with_no_schedules() {
        let mut state = WorkspaceState::new(0);
        assert_eq!(state.schedules, Vec::new());
        assert_eq!(state.schedule_stats().success_rate_pct, 0);
        assert!(
            state
                .create_schedule(
                    "Weekday briefing",
                    "every weekday at 9",
                    "Give me a short briefing of what's on today"
                )
                .is_some()
        );
        assert_eq!(state.schedules.len(), 1);
        assert_eq!(state.schedules[0].name, "Weekday briefing");
        assert_eq!(state.schedule_stats().active_schedules, 1);
    }

    #[test]
    fn background_replies_and_queued_prompts_stay_with_their_original_conversation() {
        let mut state = WorkspaceState::new_at(9, Timestamp(100));
        let first = state.session_id.clone();
        state.submit_prompt("First conversation");
        state.apply_event(&Event::AgentChunk {
            session_id: first.clone(),
            text: "First half, ".into(),
        });
        state.submit_prompt("Follow up in the first conversation");
        let second = state.start_new_chat().id;
        state.submit_prompt("Second conversation");
        let visible = state.messages.clone();
        state.observe_time(Timestamp(200));
        state.apply_event(&Event::AgentChunk {
            session_id: first.clone(),
            text: "second half.".into(),
        });
        let next = state.apply_event(&Event::AgentCompleted {
            session_id: first.clone(),
        });
        assert_eq!(state.session_id, second);
        assert_eq!(state.messages, visible);
        assert!(
            state.is_generating,
            "another chat's completion cannot stop this one"
        );
        let stored = state
            .threads
            .iter()
            .find(|thread| thread.id == first)
            .unwrap();
        assert!(
            stored
                .messages
                .iter()
                .any(|message| message.text == "First half, second half.")
        );
        let Some(crate::Command::RunPrompt {
            session_id,
            prompt,
            context: Some(context),
        }) = next
        else {
            panic!("the original chat's queued prompt must not be lost");
        };
        assert_eq!(session_id, first);
        assert_eq!(prompt, "Follow up in the first conversation");
        assert!(
            context
                .messages()
                .iter()
                .any(|message| message.text == "First half, second half.")
        );
        assert!(
            context
                .messages()
                .iter()
                .all(|message| !message.text.contains("Second conversation"))
        );
        assert_eq!(stored.updated_at, Timestamp(200));
        assert!(state.select_thread(&first));
        assert!(
            state.is_generating,
            "the queued follow-up is running in this conversation"
        );
        assert_eq!(
            state.messages.last().unwrap().text,
            "Follow up in the first conversation"
        );
        assert!(state.select_thread(&second));
        assert!(
            state.is_generating,
            "returning to an unfinished conversation restores Stop"
        );
    }

    #[test]
    fn conversation_dates_follow_observed_events_without_rewriting_creation_or_navigation() {
        let mut state = WorkspaceState::new_at(7, Timestamp(100));
        let session = state.session_id.clone();
        assert_eq!(state.active_thread().unwrap().created_at, Timestamp(100));
        state.observe_time(Timestamp(200));
        state.submit_prompt("A fictional conversation.");
        assert_eq!(state.active_thread().unwrap().updated_at, Timestamp(200));
        state.observe_time(Timestamp(300));
        state.apply_event(&Event::AgentChunk {
            session_id: session.clone(),
            text: "A fictional answer.".into(),
        });
        state.apply_event(&Event::AgentCompleted {
            session_id: session.clone(),
        });
        let completed = state.active_thread().unwrap().clone();
        assert_eq!(completed.created_at, Timestamp(100));
        assert_eq!(completed.updated_at, Timestamp(300));
        state.observe_time(Timestamp(400));
        let draft = state.start_new_chat();
        assert_eq!(draft.created_at, Timestamp(400));
        assert!(state.select_thread(&session));
        assert_eq!(state.active_thread(), Some(&completed));

        // Restored timestamps are evidence too, even if the clock has moved back.
        let mut restored = WorkspaceState::new_at(8, Timestamp(250));
        restored.apply_event(&Event::ChatHistoryLoaded {
            threads: vec![completed],
        });
        assert!(restored.select_thread(&session));
        restored.submit_prompt("A follow-up while the clock is behind.");
        assert_eq!(restored.active_thread().unwrap().updated_at, Timestamp(300));
        restored.apply_event(&Event::AgentCompleted {
            session_id: session.clone(),
        });
        restored.observe_time(Timestamp(500));
        restored.observe_time(Timestamp(350));
        restored.submit_prompt("A later follow-up.");
        assert_eq!(restored.active_thread().unwrap().created_at, Timestamp(100));
        assert_eq!(restored.active_thread().unwrap().updated_at, Timestamp(500));
        restored.observe_time(Timestamp(600));
        assert!(restored.delete_thread(&session));
        assert_eq!(
            restored
                .threads
                .iter()
                .find(|thread| thread.id == session)
                .unwrap()
                .deleted_at,
            Some(Timestamp(600))
        );
    }

    #[test]
    fn response_model_is_pinned_to_each_reply_and_saved_with_history() {
        let mut state = WorkspaceState::new(0);
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
        let mut state = WorkspaceState::new(0);
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
    fn replies_in_a_restored_conversation_never_reuse_message_ids() {
        // A fresh app session restarts the id counter; restored messages already
        // used low ids, and a reused id made the UI overwrite an old reply's text.
        // Exercise a restored active conversation before receiving more chunks.
        let mut state = WorkspaceState::new(0);
        let session = state.session_id.clone();
        let mut thread = ChatThread::new(session.clone(), Timestamp(1));
        thread
            .messages
            .push(ChatMessage::user("msg-user-1", "Hi", "Earlier"));
        thread.messages.push(ChatMessage::assistant(
            "msg-assistant-2",
            "Old answer",
            "Earlier",
            Vec::new(),
        ));
        state.threads.clear();
        state.apply_event(&Event::ChatHistoryLoaded {
            threads: vec![thread],
        });
        assert_eq!(
            state.messages.len(),
            2,
            "history restores into the startup chat"
        );
        for _ in 0..3 {
            state.submit_prompt("Again");
            state.apply_event(&Event::AgentChunk {
                session_id: session.clone(),
                text: "New answer".into(),
            });
            state.apply_event(&Event::AgentCompleted {
                session_id: session.clone(),
            });
        }
        let mut ids: Vec<_> = state.messages.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            state.messages.len(),
            "message ids must be unique"
        );
        assert_eq!(state.messages[1].text, "Old answer");
    }

    #[test]
    fn stopping_mid_answer_keeps_the_partial_reply_in_history() {
        let mut state = WorkspaceState::new(0);
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
        let mut state = WorkspaceState::new(0);
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
        let state = WorkspaceState::new(0);
        assert_eq!(state.messages, Vec::<ChatMessage>::new());
        assert_eq!(state.queued_prompts, Vec::<String>::new());
        assert!(!state.is_generating);
        assert!(!state.ai_building);
        assert!(state.last_route_evaluation.is_none());
    }

    #[test]
    fn login_and_logout_switch_routes() {
        let mut state = WorkspaceState::new(0);
        state.logout();
        assert_eq!(state.route, ScreenRoute::Login);
        let session = crate::auth::AuthSession {
            account_id: "acct-1".into(),
            email: "barney@sovra.ai".into(),
            full_name: "Barney the Jedi".into(),
            provider: crate::auth::AuthProvider::Password,
            signed_in_at: Timestamp(1),
        };
        state.apply_auth(&crate::auth::AuthEvent::SignedIn {
            session,
            new_account: false,
        });
        assert_eq!(state.route, ScreenRoute::MainWorkspace);
        assert_eq!(state.user.email, "barney@sovra.ai");
        assert_eq!(state.user.initials, "BJ");
        state.apply_auth(&crate::auth::AuthEvent::SignedOut);
        assert_eq!(state.route, ScreenRoute::Login);
        assert!(!state.auth.is_signed_in());
    }

    #[test]
    fn startup_without_an_auth_report_keeps_the_workspace_route() {
        let mut state = WorkspaceState::new(0);
        assert_eq!(state.route, ScreenRoute::MainWorkspace);
        state.apply_auth(&crate::auth::AuthEvent::Loaded {
            session: None,
            has_accounts: false,
            google_ready: false,
        });
        assert_eq!(
            state.route,
            ScreenRoute::Login,
            "no saved session gates the app"
        );
    }

    #[test]
    fn queues_prompts_while_generating_and_flushes_on_complete() {
        let mut state = WorkspaceState::new(0);
        let first = state.submit_prompt("First prompt");
        assert_eq!(first.as_deref(), Some("First prompt"));
        assert!(state.is_generating);

        let second = state.submit_prompt("Queued second prompt");
        assert!(second.is_none());
        assert_eq!(state.queued_prompts, vec!["Queued second prompt"]);

        let next = state.apply_event(&Event::AgentCompleted {
            session_id: state.session_id.clone(),
        });
        assert!(
            matches!(next, Some(crate::Command::RunPrompt { prompt, session_id, .. }) if prompt == "Queued second prompt" && session_id == state.session_id)
        );
        assert!(state.is_generating);
        assert_eq!(state.queued_prompts.len(), 0);
    }

    #[test]
    fn preview_ai_completes_deployment_and_checklist() {
        let mut state = WorkspaceState::new(0);
        state.reset_build_progress();
        assert!(state.ai_building);
        state.activate_preview_ai();
        assert!(!state.ai_building);
        assert!(state.config_cards.iter().all(|c| c.status.is_complete()));
    }
    #[test]
    fn stream_chunks_preserve_exact_markdown_and_unicode() {
        let mut state = WorkspaceState::new(0);
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
        let mut state = WorkspaceState::new(0);
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
    fn new_chats_preserve_restored_conversations_after_restart() {
        let mut state = WorkspaceState::new(0);
        state.submit_prompt("A different conversation");
        let mut saved = ChatThread::new("chat-session-2", Timestamp(1));
        saved.messages.push(ChatMessage::user(
            "msg-user-1",
            "My name is Shaun",
            "Earlier",
        ));
        state.apply_event(&Event::ChatHistoryLoaded {
            threads: vec![saved.clone()],
        });
        let fresh = state.start_new_chat();
        assert_ne!(fresh.id, saved.id, "a fresh chat must not reuse a saved id");
        state.submit_prompt("Please remember my preferred answer format");
        assert_eq!(
            state.threads.iter().find(|thread| thread.id == saved.id),
            Some(&saved),
            "sending in the fresh chat must preserve the original transcript"
        );
    }

    #[test]
    fn launch_namespaces_keep_unloaded_and_deleted_conversations_distinct() {
        let mut first = WorkspaceState::new(123);
        let mut original_ids = std::collections::BTreeSet::new();
        for _ in 0..60 {
            original_ids.insert(first.session_id.clone());
            first.submit_prompt("Remember this conversation");
            first.start_new_chat();
        }
        original_ids.insert(first.session_id.clone());
        let mut restarted = WorkspaceState::new(456);
        assert!(!original_ids.contains(&restarted.session_id));
        // A history page is incomplete: hidden/deleted and older chats still own
        // their identities even when the UI never receives them.
        restarted.apply_event(&Event::ChatHistoryLoaded {
            threads: first.threads.iter().take(50).cloned().collect(),
        });
        let original = restarted.session_id.clone();
        assert_eq!(
            restarted.start_new_chat().id,
            original,
            "reuse the empty draft"
        );
        for _ in 0..65 {
            restarted.submit_prompt("A new conversation after restarting");
            let fresh = restarted.start_new_chat();
            assert!(!original_ids.contains(&fresh.id));
        }
        // Removing every active thread creates another draft in this launch's
        // namespace, without reviving a retained deleted thread.
        let ids: Vec<_> = restarted
            .threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        for id in &ids {
            assert!(restarted.delete_thread(id));
        }
        assert!(!ids.contains(&restarted.session_id));
        assert!(!original_ids.contains(&restarted.session_id));
        assert!(restarted.active_thread().unwrap().is_untouched_draft());
    }

    #[test]
    fn deleting_the_last_chat_does_not_reuse_a_retained_deleted_id() {
        let mut state = WorkspaceState::new(0);
        let original = state.session_id.clone();
        let mut deleted = ChatThread::new("chat-session-2", Timestamp(1));
        deleted.deleted_at = Some(Timestamp(2));
        state.threads.push(deleted.clone());
        assert!(state.delete_thread(&original));
        assert_ne!(state.session_id, original);
        assert_ne!(state.session_id, deleted.id);
        assert!(state.active_thread().unwrap().is_untouched_draft());
    }

    #[test]
    fn manages_multiple_chat_threads_and_scheduled_runs() {
        let mut state = WorkspaceState::new(0);
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

    #[test]
    fn tour_lifecycle_navigates_across_all_five_tabs_and_completes() {
        let mut state = WorkspaceState::new(0);
        assert_eq!(state.tour_step, None);
        assert!(!state.onboarding_completed);

        assert_eq!(state.start_tour(), TourStep::WelcomeChat);
        assert_eq!(state.active_tab, NavTab::Chat);

        assert_eq!(state.next_tour_step(), Some(TourStep::AutomaticRouting));
        assert_eq!(state.active_tab, NavTab::MyAis);

        assert_eq!(state.prev_tour_step(), Some(TourStep::WelcomeChat));
        assert_eq!(state.active_tab, NavTab::Chat);

        assert_eq!(state.next_tour_step(), Some(TourStep::AutomaticRouting));
        assert_eq!(state.next_tour_step(), Some(TourStep::KnowledgeMemory));
        assert_eq!(state.active_tab, NavTab::Knowledge);

        assert_eq!(state.next_tour_step(), Some(TourStep::AutomatedSchedules));
        assert_eq!(state.active_tab, NavTab::Schedules);

        assert_eq!(state.next_tour_step(), Some(TourStep::PersonalizeSettings));
        assert_eq!(state.active_tab, NavTab::Settings);

        assert_eq!(state.next_tour_step(), None);
        assert_eq!(state.tour_step, None);
        assert!(state.onboarding_completed);
        assert_eq!(state.active_tab, NavTab::Chat);
    }

    #[test]
    fn a_message_sent_before_setup_is_sent_once_a_model_is_chosen() {
        let mut state = WorkspaceState::new(0);
        let mut router =
            crate::models::ModelConnection::new("router", crate::models::ProviderKind::OpenRouter);
        router.has_credential = true;
        state.apply_event(&Event::Models(crate::models::ModelEvent::Loaded {
            connections: vec![router],
            preferences: crate::models::ModelPreferences::default(),
        }));
        assert!(state.submit_prompt("What's the weather?").is_none());
        assert_eq!(state.messages.len(), 2, "the message and a setup card");

        let key = ModelKey {
            connection_id: "router".into(),
            model_id: "chat".into(),
        };
        let preferences = crate::models::ModelPreferences {
            active: Some(key),
            ..crate::models::ModelPreferences::default()
        };
        let resent = state.apply_event(&Event::Models(
            crate::models::ModelEvent::PreferencesChanged(preferences.clone()),
        ));
        assert!(
            matches!(resent, Some(crate::Command::RunPrompt { prompt, session_id, .. }) if prompt == "What's the weather?" && session_id == state.session_id)
        );
        assert!(state.is_generating);
        assert_eq!(state.messages.len(), 1, "the setup card is gone");
        assert!(
            state
                .apply_event(&Event::Models(
                    crate::models::ModelEvent::PreferencesChanged(preferences)
                ))
                .is_none(),
            "it is sent only once"
        );
    }

    #[test]
    fn a_chosen_model_the_runtime_cannot_run_is_not_ready() {
        let mut state = WorkspaceState::new(0);
        let mut router =
            crate::models::ModelConnection::new("router", crate::models::ProviderKind::OpenRouter);
        router.has_credential = true;
        state.apply_event(&Event::Models(crate::models::ModelEvent::Loaded {
            connections: vec![router],
            preferences: crate::models::ModelPreferences {
                active: Some(ModelKey {
                    connection_id: "router".into(),
                    model_id: "gone".into(),
                }),
                ..crate::models::ModelPreferences::default()
            },
        }));
        assert!(state.has_active_llm);
        state.apply_event(&Event::Models(
            crate::models::ModelEvent::ActiveModelStatus {
                problem: Some("it is no longer offered by this service.".into()),
            },
        ));
        assert!(!state.has_active_llm);
        assert!(matches!(
            state.setup_stage,
            crate::models::SetupStage::NeedsModelSelection { .. }
        ));
        assert!(
            state
                .llm_setup_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("no longer offered"))
        );
        state.apply_event(&Event::Models(
            crate::models::ModelEvent::ActiveModelStatus { problem: None },
        ));
        assert!(state.has_active_llm);
    }

    #[test]
    fn prompt_with_connected_key_only_shows_model_selection_setup_card() {
        let mut state = WorkspaceState::new(0);
        let mut router =
            crate::models::ModelConnection::new("router", crate::models::ProviderKind::OpenRouter);
        router.has_credential = true;
        router.key_preview = "sk-or-••••1234".into();
        state.apply_event(&Event::Models(crate::models::ModelEvent::Loaded {
            connections: vec![router],
            preferences: crate::models::ModelPreferences::default(),
        }));
        assert!(state.models_loaded);
        assert!(!state.has_active_llm);
        assert_eq!(
            state.setup_stage,
            crate::models::SetupStage::NeedsModelSelection {
                connected_service: "OpenRouter".into(),
                key_preview: Some("sk-or-••••1234".into()),
            }
        );

        let dispatched = state.submit_prompt("Hello Sovra!");
        assert!(dispatched.is_none());
        let assistant = state.messages.last().unwrap();
        let debug_repr = format!("{:?}", assistant.widgets);
        assert!(
            debug_repr.contains("Step 2 of 2"),
            "must show Step 2 model selection: {debug_repr}"
        );
        assert!(
            debug_repr.contains("setup:recommended-models"),
            "must offer recommended models action: {debug_repr}"
        );
        assert!(
            !debug_repr.contains("setup:openrouter-key"),
            "must NOT ask for OpenRouter key when key is already set: {debug_repr}"
        );
        assert!(
            !debug_repr.contains("setup:ollama"),
            "must NOT show Ollama setup when key is already set: {debug_repr}"
        );
    }

    #[test]
    fn window_title_names_the_page_before_the_product() {
        assert_eq!(
            window_title(
                ScreenRoute::MainWorkspace,
                NavTab::Knowledge,
                "Sovra",
                &crate::models::UiLanguage::english(),
            ),
            "Knowledge — Sovra"
        );
        assert_eq!(
            window_title(
                ScreenRoute::MainWorkspace,
                NavTab::Knowledge,
                "Sovra",
                &crate::models::UiLanguage::arabic(),
            ),
            "المعرفة — Sovra"
        );
        assert_eq!(
            window_title(
                ScreenRoute::Login,
                NavTab::Knowledge,
                "Sovra",
                &crate::models::UiLanguage::default(),
            ),
            "Sovra"
        );
    }
}
