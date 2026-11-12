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
    /// Live team card: AG-UI `ACTIVITY_SNAPSHOT` content for a team run.
    TeamActivity(crate::teams::TeamActivity),
    /// In-chat research kanban: waiting / working / done lanes for deep research.
    ResearchBoard(crate::research::ResearchBoard),
    /// Source preview while code is being written; never executed in the app.
    CodeWriting {
        language: String,
        content: String,
        /// Present only after the storage port acknowledges this source.
        #[serde(default)]
        artifact_id: Option<String>,
    },
    /// The live agent-loop log for this reply: each step, helpers side by side.
    WorkLog(crate::work::WorkActivity),
    /// An artifact an agent saved during the turn, shown as a card to open it.
    ArtifactSaved {
        artifact_id: String,
        title: String,
        artifact_kind: crate::artifacts::ArtifactKind,
    },
}

/// Live activity payload for AG-UI `ACTIVITY_SNAPSHOT` (team card or research board).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "activity_type", rename_all = "snake_case")]
pub enum LiveActivity {
    /// Moderated team chat (`activityType: "sovra.team"`).
    #[serde(rename = "sovra.team")]
    Team(crate::teams::TeamActivity),
    /// Parallel deep research board (`activityType: "sovra.research"`).
    #[serde(rename = "sovra.research")]
    Research(crate::research::ResearchBoard),
    /// The agent loop's live steps (`activityType: "sovra.work"`).
    #[serde(rename = "sovra.work")]
    Work(crate::work::WorkActivity),
    #[serde(rename = "sovra.code")]
    CodeWriting {
        language: String,
        content: String,
        /// Present only after the storage port acknowledges this source.
        #[serde(default)]
        artifact_id: Option<String>,
    },
}

/// Who wrote a message when it was not the default assistant: a team member.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Speaker {
    pub member_id: String,
    pub name: String,
    pub icon: String,
    pub role: String,
}

/// How an AG-UI subagent invocation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentOutcome {
    Success,
    /// The member chose not to speak (`<pass/>`); nothing was shown.
    Passed,
    Failed,
}

/// The AG-UI protocol events Sovra streams for multi-agent runs, typed instead of
/// JSON (<https://docs.ag-ui.com/concepts/events>). Field names follow the spec;
/// `subagent_run_id` attributes a message to one team member's invocation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgUiEvent {
    RunStarted {
        run_id: String,
    },
    /// Replaces the activity message `message_id` (team card or research board).
    ActivitySnapshot {
        message_id: String,
        activity: LiveActivity,
    },
    SubagentStarted {
        subagent_run_id: String,
        /// The member handle; `speaker` carries how to show it.
        name: String,
        description: String,
        speaker: Speaker,
    },
    TextMessageStart {
        message_id: String,
        subagent_run_id: Option<String>,
    },
    TextMessageContent {
        message_id: String,
        delta: String,
    },
    TextMessageEnd {
        message_id: String,
    },
    SubagentFinished {
        subagent_run_id: String,
        outcome: SubagentOutcome,
    },
    RunFinished {
        run_id: String,
    },
    RunError {
        message: String,
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
    /// An immutable saved source, attached only after durable admission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::EpisodeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<crate::Timestamp>,
    pub widgets: Vec<AgUiWidget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::files::Attachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ResponseModel>,
    /// The model's visible reasoning, streamed before (or between) answer text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    /// How long the model had been working when its latest reasoning arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_ms: Option<u64>,
    /// The team member who wrote this reply; `None` for Sovra itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<Speaker>,
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
            source: None,
            occurred_at: None,
            widgets: Vec::new(),
            attachments: Vec::new(),
            model: None,
            thinking: String::new(),
            thinking_ms: None,
            speaker: None,
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
            source: None,
            occurred_at: None,
            widgets,
            attachments: Vec::new(),
            model: None,
            thinking: String::new(),
            thinking_ms: None,
            speaker: None,
        }
    }
}

/// Surface ID for the default AG-UI onboarding card shown when no LLM is active.
pub const LLM_SETUP_SURFACE_ID: &str = "llm-setup-guide";

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

/// Default AG-UI surface shown in chat when the user sends a message before any LLM is active.
///
/// Shows the runtime's explanation and a single action to resume guided setup.
/// Without saved connection state, it must not guess which service needs a key.
pub fn default_llm_setup_widget(status_hint: Option<&str>) -> AgUiWidget {
    // An error message is not connection state. The Models page derives the
    // next action from saved preferences instead of guessing from wording.
    AgUiWidget::Components {
        surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
        version: 3,
        components: vec![UiComponent::Card {
            title: "Check your AI setup".to_owned(),
            description: status_hint.unwrap_or("Connect an AI service and choose a model to start chatting. Sovra will guide you through the remaining steps.").to_owned(),
            children: vec![UiComponent::Button {
                label: "Continue setup →".to_owned(),
                action_id: "setup:recommended-models".to_owned(),
                disabled: false,
            }],
        }],
    }
}

/// State-aware AG-UI setup card that ONLY displays the remaining setup step(s).
///
/// - When a key is ALREADY configured (`SetupStage::NeedsModelSelection`), it hides
///   all connection setup options and exclusively guides the user through Step 2: picking a model.
/// - When a model is selected but its connection lacks a key (`SetupStage::NeedsModelKey`),
///   it only asks for that connection's key.
/// - When nothing is connected yet (`SetupStage::NeedsConnection`), it walks through
///   Step 1 with a 1-click browser link to `openrouter.ai/keys`.
pub fn llm_setup_widget_for_stage(
    stage: &crate::models::SetupStage,
    custom_hint: Option<&str>,
) -> AgUiWidget {
    match stage {
        crate::models::SetupStage::NeedsModelSelection {
            connected_service, ..
        } => {
            let hint = custom_hint.map_or_else(
                || format!("✓ {connected_service} is connected. One step left."),
                str::to_owned,
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 2,
                components: vec![UiComponent::Card {
                    title: "Step 2 of 2 · Pick the AI you want to talk to".to_owned(),
                    description: "Choose one of our suggestions. You'll come straight back here to chat, and your message will be ready to send again.".to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "Almost there".to_owned(),
                            description: hint,
                        },
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: "Pick an AI →".to_owned(),
                                action_id: "setup:recommended-models".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "See all chat models".to_owned(),
                                action_id: "setup:choose-model".to_owned(),
                                disabled: false,
                            },
                        ]),
                    ],
                }],
            }
        }
        crate::models::SetupStage::NeedsModelKey {
            model_id,
            connection_name,
            ..
        } => {
            let hint = custom_hint.map_or_else(
                || {
                    format!(
                        "You chose “{model_id}”. {connection_name} needs your key so it can reply."
                    )
                },
                str::to_owned,
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 2,
                components: vec![UiComponent::Card {
                    title: format!("Almost ready · Add your {connection_name} key"),
                    description: "Paste your key once and you can start chatting straight away."
                        .to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "Your AI is chosen — it just needs your key".to_owned(),
                            description: hint,
                        },
                        UiComponent::Row(vec![UiComponent::Button {
                            label: format!("Add {connection_name} key →"),
                            action_id: "setup:selected-key".to_owned(),
                            disabled: false,
                        }]),
                    ],
                }],
            }
        }
        crate::models::SetupStage::Loading => AgUiWidget::SurfaceCard {
            surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
            title: "Checking your saved setup…".to_owned(),
            summary: "Your saved connections and model choice are loading.".to_owned(),
        },
        crate::models::SetupStage::Ready { active } => AgUiWidget::SurfaceCard {
            surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
            title: "Your AI choice is saved".to_owned(),
            summary: format!("{} is selected. Send a message to try it.", active.model_id),
        },
        crate::models::SetupStage::NeedsConnection => {
            let hint = custom_hint.unwrap_or(
                "1. Get a free key: a website opens where you sign up and copy a key.\n\
                 2. Paste it here, and Sovra is ready to answer.",
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 1,
                components: vec![UiComponent::Card {
                    title: "First, connect an AI".to_owned(),
                    description: "Sovra needs an AI to answer you. The easiest way takes about \
                        two minutes and has free choices. Your message is kept and sent as soon \
                        as you're done."
                        .to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "Two steps".to_owned(),
                            description: hint.to_owned(),
                        },
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: "1. Get a free key (opens a website)".to_owned(),
                                action_id: "setup:open-openrouter-web".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "2. Paste my key".to_owned(),
                                action_id: "setup:openrouter-key".to_owned(),
                                disabled: false,
                            },
                        ]),
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: "Use a free AI on this computer instead".to_owned(),
                                action_id: "setup:ollama".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "Other ways to connect".to_owned(),
                                action_id: "setup:other-ways".to_owned(),
                                disabled: false,
                            },
                        ]),
                    ],
                }],
            }
        }
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

#[cfg(test)]
mod setup_tests {
    use super::*;
    use crate::models::{ModelKey, SetupStage};

    #[test]
    fn loading_and_ready_do_not_ask_to_connect_again() {
        for stage in [
            SetupStage::Loading,
            SetupStage::Ready {
                active: ModelKey {
                    connection_id: "custom".into(),
                    model_id: "chat".into(),
                },
            },
        ] {
            assert!(matches!(
                llm_setup_widget_for_stage(&stage, None),
                AgUiWidget::SurfaceCard { .. }
            ));
        }
    }

    #[test]
    fn custom_key_setup_and_runtime_errors_do_not_guess_openrouter() {
        let widget = llm_setup_widget_for_stage(
            &SetupStage::NeedsModelKey {
                model_id: "chat".into(),
                connection_id: "custom".into(),
                connection_name: "My service".into(),
            },
            None,
        );
        let description = format!("{widget:?}");
        assert!(description.contains("setup:selected-key"));
        assert!(!description.contains("openrouter"));
        let fallback = default_llm_setup_widget(Some(
            "Step 1 complete, but key is connected to a different server",
        ));
        let description = format!("{fallback:?}");
        assert!(description.contains("setup:recommended-models"));
        assert!(!description.contains("OpenRouter"));
        assert!(!description.contains("setup:openrouter-key"));
    }
}
