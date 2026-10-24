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
    Models(crate::models::ModelCommand),
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
    /// Assign a specific model override or preferred tier to one of the 22 `SovraSRX` specialist agents.
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
        session_id: String,
        prompt: String,
    },
    /// Interrupt an active agent run for the given session.
    InterruptSession {
        session_id: String,
    },
    /// Stop the runtime; it answers with [`Event::Stopped`].
    Shutdown,
}

/// A notification from the runtime to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Models(crate::models::ModelEvent),
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
    /// The runtime has stopped; no further events follow.
    Stopped,
}
