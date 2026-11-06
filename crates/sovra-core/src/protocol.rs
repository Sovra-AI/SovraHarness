//! The message protocol between the UI and the runtime.
//!
//! Enums are deliberately exhaustive: adding a variant must fail compilation at every
//! `match` that has to handle it.

use crate::agui::AgUiWidget;
use crate::chat_history::ChatThread;
use crate::models::ModelKey;
use crate::providers::ModelTier;
use crate::routing_tree::{DecisionBackend, RouteEvaluation, SelectionMode};

/// A request from the UI to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Development(crate::development::DevelopmentCommand),
    Auth(crate::auth::AuthCommand),
    Memory(crate::MemoryCommand),
    Models(crate::models::ModelCommand),
    Files(crate::files::FileCommand),
    FileIndex(crate::file_index::IndexCommand),
    Skills(crate::skills::SkillCommand),
    Teams(crate::teams::TeamCommand),
    Artifacts(crate::artifacts::ArtifactCommand),
    Connections(crate::connections::ConnectionCommand),
    /// The Google account on the Connections page.
    Google(crate::google::GoogleCommand),
    Mcp(crate::mcp::McpCommand),
    /// Terminal approvals and settings.
    Terminal(crate::terminal::TerminalCommand),
    Schedule(crate::schedules::ScheduleCommand),
    /// Groups of people and their computers.
    Group(crate::group::GroupCommand),
    RunWithFiles {
        /// Explicit earlier text replaces in-process history; `None` keeps that history.
        context: Option<crate::ConversationContext>,
        session_id: String,
        prompt: String,
        attachment_ids: Vec<String>,
    },
    /// Load persisted conversation threads from the memory store.
    LoadChatHistory,
    /// Upsert a conversation thread (metadata + messages) to persistent storage.
    SaveChatThread {
        thread: Box<ChatThread>,
    },
    /// Soft-delete a conversation thread in persistent storage.
    DeleteChatThread {
        session_id: String,
    },
    /// Update the active model routing mode or System-1 decision backend (`Laya`, `OpenJev`, or `EmbeddedHeuristic`).
    UpdateRouting {
        mode: SelectionMode,
        backend: DecisionBackend,
        global_model: Option<ModelKey>,
    },
    /// Assign a specific model override or preferred tier to one of the `SovraSRX` specialist helpers.
    SetSpecialistModel {
        specialist_id: String,
        model_override: Option<ModelKey>,
        preferred_tier: ModelTier,
    },
    /// Simulate a prompt through the active routing tree (calling Laya/OpenJev or the embedded engine) without running the target LLM.
    SimulateRoute {
        prompt: String,
    },
    /// Liveness probe; the runtime answers with [`Event::Pong`].
    Ping,
    /// Send a user prompt to the active ADK session.
    RunPrompt {
        /// Explicit earlier text replaces in-process history; `None` keeps that history.
        context: Option<crate::ConversationContext>,
        session_id: String,
        prompt: String,
    },
    /// Interrupt an active agent run for the given session.
    InterruptSession {
        session_id: String,
    },
    /// Ask the active model to generate a concise conversation title for `session_id`.
    GenerateChatTitle {
        session_id: String,
        prompt: String,
        reply: String,
    },
    /// Stop the runtime; it answers with [`Event::Stopped`].
    Shutdown,
}

/// A notification from the runtime to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Development(crate::development::DevelopmentEvent),
    Auth(crate::auth::AuthEvent),
    Memory(crate::MemoryEvent),
    Models(crate::models::ModelEvent),
    FilePrepared(crate::files::FilePrepared),
    FileIndex(crate::file_index::IndexEvent),
    Skills(crate::skills::SkillEvent),
    Teams(crate::teams::TeamEvent),
    Artifacts(crate::artifacts::ArtifactEvent),
    Connections(crate::connections::ConnectionEvent),
    Google(crate::google::GoogleEvent),
    Mcp(crate::mcp::McpEvent),
    /// Terminal cards and settings. Not session-scoped: the state is shared and each
    /// call names its own conversation, so cards for background chats are kept too.
    Terminal(crate::terminal::TerminalEvent),
    Schedule(crate::schedules::ScheduleEvent),
    /// Groups of people and their computers. Boxed: a roster is large next to the
    /// other events, and every event is queued by value.
    Group(Box<crate::group::GroupEvent>),
    /// An AG-UI protocol event for `session_id` (team runs stream through these).
    AgUi {
        session_id: String,
        event: crate::agui::AgUiEvent,
    },
    /// Persisted conversation threads loaded from the memory store.
    ChatHistoryLoaded {
        threads: Vec<ChatThread>,
    },
    /// The runtime is up and accepting commands.
    Ready {
        version: String,
    },
    /// Reply to [`Command::Ping`].
    Pong,
    /// Emitted when the routing tree or mode evaluates a prompt (either via simulation or before a live turn).
    RouteEvaluated {
        evaluation: Box<RouteEvaluation>,
    },
    /// Execution identity, sent once immediately before the first assistant output.
    AgentResponseStarted {
        session_id: String,
        model: crate::ResponseModel,
    },
    /// A streamed text chunk emitted by the ADK agent for a session.
    AgentChunk {
        session_id: String,
        text: String,
    },
    /// A streamed reasoning chunk. `elapsed_ms` is measured from the start of the turn,
    /// so the UI can say how long the model thought without reading a clock itself.
    AgentThinkingChunk {
        session_id: String,
        text: String,
        elapsed_ms: u64,
    },
    /// A structured AG-UI surface emitted by `adk-ui` or the agent runner.
    AgUiSurface {
        session_id: String,
        widget: AgUiWidget,
    },
    /// The ADK agent finished responding for a session.
    AgentCompleted {
        session_id: String,
    },
    /// An agent invocation or configuration error occurred for a session.
    AgentError {
        session_id: String,
        message: String,
    },
    /// A concise conversation title generated by the model for `session_id`.
    ChatTitleGenerated {
        session_id: String,
        title: String,
    },
    /// The runtime has stopped; no further events follow.
    Stopped,
}

impl Event {
    /// The chat session this event belongs to, when it belongs to one. The runtime uses
    /// it to keep events for sessions that answer other channels (`WhatsApp`) off the
    /// chat window.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::AgUi { session_id, .. }
            | Self::AgentResponseStarted { session_id, .. }
            | Self::AgentChunk { session_id, .. }
            | Self::AgentThinkingChunk { session_id, .. }
            | Self::AgUiSurface { session_id, .. }
            | Self::AgentCompleted { session_id }
            | Self::AgentError { session_id, .. }
            | Self::ChatTitleGenerated { session_id, .. }
            | Self::Memory(crate::MemoryEvent::SourceSaved { session_id, .. }) => Some(session_id),
            Self::FilePrepared(prepared) => Some(&prepared.session_id),
            Self::Development(_)
            | Self::Auth(_)
            | Self::Memory(_)
            | Self::Models(_)
            | Self::FileIndex(_)
            | Self::Skills(_)
            | Self::Teams(_)
            | Self::Artifacts(_)
            | Self::Connections(_)
            | Self::Google(_)
            | Self::Mcp(_)
            | Self::Terminal(_)
            | Self::Schedule(_)
            | Self::Group(_)
            | Self::ChatHistoryLoaded { .. }
            | Self::Ready { .. }
            | Self::Pong
            | Self::RouteEvaluated { .. }
            | Self::Stopped => None,
        }
    }
}
