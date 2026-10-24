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
    /// The model's visible reasoning, streamed before (or between) answer text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thinking: String,
    /// How long the model had been working when its latest reasoning arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_ms: Option<u64>,
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
            thinking: String::new(),
            thinking_ms: None,
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
            thinking: String::new(),
            thinking_ms: None,
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
            connected_service,
            key_preview,
        } => {
            let key_badge = key_preview
                .as_deref()
                .map_or_else(String::new, |p| format!(" · Active key: {p}"));
            let hint = custom_hint.map_or_else(
                || {
                    format!(
                        "✓ Step 1 Complete: {connected_service} is connected{key_badge}. You only have 1 step left!"
                    )
                },
                str::to_owned,
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 2,
                components: vec![UiComponent::Card {
                    title: "Step 2 of 2 · Pick Your AI Model to Start Chatting".to_owned(),
                    description: "Your connection is saved. Choose an AI to answer your messages. You'll see its price before choosing; a free option appears when the service lists one.".to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "✓ Step 1 Complete — API Key Connected".to_owned(),
                            description: hint,
                        },
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: "Step 2: Pick a Recommended Model →".to_owned(),
                                action_id: "setup:recommended-models".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "Browse All Models".to_owned(),
                                action_id: "setup:choose-model".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "Take Interactive System Tour".to_owned(),
                                action_id: "setup:start-tour".to_owned(),
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
                        "You selected “{model_id}”, which runs on {connection_name}. Add your API key to finish setup."
                    )
                },
                str::to_owned,
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 2,
                components: vec![UiComponent::Card {
                    title: format!("Almost Ready · Add Key for {connection_name}"),
                    description: "Your model is already chosen! Just paste your API key so Sovra can talk to it.".to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "Model Selected — Waiting for API Key".to_owned(),
                            description: hint,
                        },
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: format!("Add {connection_name} key →"),
                                action_id: "setup:selected-key".to_owned(),
                                disabled: false,
                            },
                        ]),
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
                "Step 1 of 2: Connect your AI service. Pick whichever option below suits you best — no technical experience needed.",
            );
            AgUiWidget::Components {
                surface_id: LLM_SETUP_SURFACE_ID.to_owned(),
                version: 1,
                components: vec![UiComponent::Card {
                    title: "Welcome to Sovra · 2-Minute Guided Setup".to_owned(),
                    description: "Before Sovra can answer your message, let's connect its AI brain in 2 quick steps: (1) Connect an AI service, then (2) Pick a recommended model.".to_owned(),
                    children: vec![
                        UiComponent::Alert {
                            title: "Current setup status".to_owned(),
                            description: hint.to_owned(),
                        },
                        UiComponent::Card {
                            title: "Option 1 · Online with an OpenRouter Key (Recommended for Everyone)".to_owned(),
                            description: "What is OpenRouter? It is a single free account that gives Sovra access to 300+ top AI models (ChatGPT, Claude, Gemini, plus 100% free starter models).\n• Step A: Click “Get Free Key at openrouter.ai” below to open the Keys page in your browser and sign in.\n• Step B: Click “Create Key”, copy the key (starts with sk-or-), and click “Paste OpenRouter Key” below.".to_owned(),
                            children: vec![UiComponent::Row(vec![
                                UiComponent::Button {
                                    label: "1. Get Free Key at openrouter.ai/keys ↗".to_owned(),
                                    action_id: "setup:open-openrouter-web".to_owned(),
                                    disabled: false,
                                },
                                UiComponent::Button {
                                    label: "2. Paste OpenRouter Key in Settings →".to_owned(),
                                    action_id: "setup:openrouter-key".to_owned(),
                                    disabled: false,
                                },
                            ])],
                        },
                        UiComponent::Card {
                            title: "Option 2 · Free & Private on Your Computer with Ollama".to_owned(),
                            description: "Run AI 100% on your own computer for free with no API key needed. 1) Download and open the free Ollama app from ollama.com. 2) Click the button below to download a local model (such as qwen3:8b) and click “Use this model”.".to_owned(),
                            children: vec![UiComponent::Button {
                                label: "Set Up Ollama (Free Local AI)".to_owned(),
                                action_id: "setup:ollama".to_owned(),
                                disabled: false,
                            }],
                        },
                        UiComponent::Card {
                            title: "Option 3 · Standard OpenAI-Compatible Server URL".to_owned(),
                            description: "Running LM Studio (http://localhost:1234/v1), vLLM (http://localhost:8000/v1), llama.cpp (http://localhost:8080/v1), or a company OpenAI-standard endpoint? Click below to enter its base URL (ending in /v1), optional API key, and activate its model.".to_owned(),
                            children: vec![UiComponent::Button {
                                label: "Add OpenAI-Compatible URL".to_owned(),
                                action_id: "setup:openai-url".to_owned(),
                                disabled: false,
                            }],
                        },
                        UiComponent::Row(vec![
                            UiComponent::Button {
                                label: "Choose Active Model in Models".to_owned(),
                                action_id: "setup:choose-model".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "Take Interactive System Tour".to_owned(),
                                action_id: "setup:start-tour".to_owned(),
                                disabled: false,
                            },
                            UiComponent::Button {
                                label: "Open All AI Settings".to_owned(),
                                action_id: "setup:settings".to_owned(),
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
