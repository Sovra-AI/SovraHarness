//! The message protocol between the UI and the runtime.
//!
//! Enums are deliberately exhaustive: adding a variant must fail compilation at every
//! `match` that has to handle it.

use crate::agui::AgUiWidget;

/// A request from the UI to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Models(crate::models::ModelCommand),
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
    /// The runtime is up and accepting commands.
    Ready {
        version: String,
    },
    /// Reply to [`Command::Ping`].
    Pong,
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
