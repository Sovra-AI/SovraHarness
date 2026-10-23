//! Native AG-UI (Agent-User Interaction) domain types and chat message primitives.
//!
//! `sovra-adk` translates `adk-ui` (`ag_ui` / `a2ui`) surfaces and `adk-core` events
//! into these pure, deterministic types so `sovra-ui` can render them natively in GPUI.

/// Role of a participant in a conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
}

/// Execution state of a step inside an AG-UI build or workflow checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildStepState {
    Pending,
    InProgress,
    Completed,
}

/// A single step inside an AG-UI live progress checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildStep {
    pub label: String,
    pub state: BuildStepState,
}

/// Interactive clarification prompt chip rendered inline in AG-UI streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClarifyChip {
    pub label: String,
    pub prompt: String,
}

/// Native AG-UI surface widget attached to an assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgUiWidget {
    /// Live "Building your personal AI..." progress card from the main chat design.
    BuildProgress {
        title: String,
        subtitle: String,
        steps: Vec<BuildStep>,
        footer_quote: String,
    },
    /// Interactive follow-up chips (`[[CLARIFY]]` / `ag_ui` prompt suggestions).
    ClarifyChips {
        question: String,
        chips: Vec<ClarifyChip>,
    },
    /// Structured status / surface update emitted via `adk-ui` (`ag_ui` profile).
    SurfaceCard {
        surface_id: String,
        title: String,
        summary: String,
    },
}

/// A single message in the main chat conversation stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub author: String,
    pub timestamp: String,
    pub text: String,
    pub widgets: Vec<AgUiWidget>,
}

impl ChatMessage {
    /// Constructs a new user message bubble.
    pub fn user(
        id: impl Into<String>,
        text: impl Into<String>,
        timestamp: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            role: MessageRole::User,
            author: "AA".to_owned(),
            timestamp: timestamp.into(),
            text: text.into(),
            widgets: Vec::new(),
        }
    }

    /// Constructs a new SOVRA assistant message with optional AG-UI widgets.
    pub fn assistant(
        id: impl Into<String>,
        text: impl Into<String>,
        timestamp: impl Into<String>,
        widgets: Vec<AgUiWidget>,
    ) -> Self {
        Self {
            id: id.into(),
            role: MessageRole::Assistant,
            author: "SOVRA".to_owned(),
            timestamp: timestamp.into(),
            text: text.into(),
            widgets,
        }
    }
}

/// Default initial AG-UI build progress widget matching the main chat design.
pub fn default_personal_ai_build_widget() -> AgUiWidget {
    AgUiWidget::BuildProgress {
        title: "Building your personal AI...".to_owned(),
        subtitle: "Analyzing your inputs and configuring your AI in real time.".to_owned(),
        steps: vec![
            BuildStep {
                label: "Understanding your goals and preferences".to_owned(),
                state: BuildStepState::Completed,
            },
            BuildStep {
                label: "Connecting and processing your knowledge".to_owned(),
                state: BuildStepState::Completed,
            },
            BuildStep {
                label: "Learning your writing style".to_owned(),
                state: BuildStepState::Completed,
            },
            BuildStep {
                label: "Configuring behaviour and guardrails".to_owned(),
                state: BuildStepState::Completed,
            },
            BuildStep {
                label: "Finalizing and optimizing...".to_owned(),
                state: BuildStepState::InProgress,
            },
        ],
        footer_quote: "\"Your AI is almost ready.\"".to_owned(),
    }
}

/// Default seeded conversation matching the Personal AI setup flow in the design.
pub fn seeded_onboarding_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::user(
            "msg-user-1",
            "I want a personal AI that knows my work, preferences and writing style. Can you help me set that up?",
            "9:41 AM",
        ),
        ChatMessage::assistant(
            "msg-assistant-1",
            "Absolutely. I'll help you build your personal SOVRA — an AI that learns from your work, remembers what matters, and communicates in your style. Let's make it yours.",
            "9:41 AM",
            vec![default_personal_ai_build_widget()],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_messages_match_personal_ai_onboarding_flow() {
        let messages = seeded_onboarding_messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[0].author, "AA");
        assert_eq!(messages[1].role, MessageRole::Assistant);
        assert_eq!(messages[1].widgets.len(), 1);
    }
}
