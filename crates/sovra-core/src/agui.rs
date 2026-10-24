//! Native AG-UI (Agent-User Interaction) domain types and chat message primitives.
//!
//! `sovra-adk` translates `adk-ui` (`ag_ui` / `a2ui`) surfaces and `adk-core` events
//! into these pure, deterministic types so `sovra-ui` can render them natively in GPUI.

/// Role of a participant in a conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

/// Execution state of a step inside an AG-UI build or workflow checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStepState {
    Pending,
    InProgress,
    Completed,
}

/// A single step inside an AG-UI live progress checklist.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildStep {
    pub label: String,
    pub state: BuildStepState,
}

/// Interactive clarification prompt chip rendered inline in AG-UI streams.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClarifyChip {
    pub label: String,
    pub prompt: String,
}

/// Native AG-UI surface widget attached to an assistant message.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgUiWidget {
    /// A complete ADK-UI surface delivered over the AG-UI custom event profile.
    Components {
        surface_id: String,
        version: u64,
        components: Vec<UiComponent>,
    },
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

/// Immutable execution identity retained with a reply, independently of current selection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResponseModel {
    pub model_id: String,
    pub name: String,
    pub provider: String,
}

/// A single message in the main chat conversation stream.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub author: String,
    pub timestamp: String,
    pub text: String,
    pub widgets: Vec<AgUiWidget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ResponseModel>,
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
            model: None,
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
            model: None,
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

/// Native component vocabulary; no GUI or ADK dependency crosses the domain boundary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiComponent {
    Text {
        text: String,
        heading: bool,
    },
    Card {
        title: String,
        description: String,
        children: Vec<Self>,
    },
    Row(Vec<Self>),
    Column(Vec<Self>),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Code(String),
    Badge(String),
    Alert {
        title: String,
        description: String,
    },
    Progress {
        label: String,
        value: u8,
    },
    Button {
        label: String,
        action_id: String,
        disabled: bool,
    },
    Divider,
}
