//! ADK-Rust adapter crate for `SovraHarness`.
//!
//! Encapsulates `adk-core`, `adk-agent`, `adk-model`, `adk-runner`, `adk-session`,
//! `adk-tool`, and `adk-ui` (native AG-UI / A2UI surfaces) behind a pure `sovra-core`
//! boundary so upstream crates never depend on ADK types directly.

pub mod decision_client;

use std::collections::HashMap;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, Llm, Part, Tool};
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{CreateRequest, GetRequest, InMemorySessionService, SessionService};
use adk_tool::GoogleSearchTool;
use adk_ui::{ACTIVE_SURFACE_STATE_KEY, UI_AGENT_PROMPT_SHORT, UiToolset};
use async_channel::Sender;
use futures::StreamExt as _;
use sovra_core::models::{Authentication, ModelConnection, ModelInfo, ModelSecret, ProviderKind};
use sovra_core::{AgUiWidget, AgentConfig, Event};

const DEFAULT_USER_ID: &str = "local-user";

/// Errors returned by the `sovra-adk` adapter boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing Gemini API key; set GEMINI_API_KEY or GOOGLE_API_KEY")]
    MissingApiKey,
    #[error("failed to initialize ADK Gemini model: {0}")]
    ModelInit(#[source] adk_core::AdkError),
    #[error("failed to build ADK agent: {0}")]
    AgentBuild(#[source] adk_core::AdkError),
    #[error("failed to build ADK runner: {0}")]
    RunnerBuild(#[source] adk_core::AdkError),
    #[error("ADK session failure: {0}")]
    Session(#[source] adk_core::AdkError),
    #[error("ADK execution failure: {0}")]
    Execution(#[source] adk_core::AdkError),
    #[error("Laya/OpenJev decision router unavailable: {0}")]
    DecisionRouter(String),
}

/// High-level ADK-Rust harness managing sessions, AG-UI tools, and agent execution.
#[derive(Clone)]
pub struct AdkHarness {
    app_name: String,
    runner: Arc<Runner>,
    sessions: Arc<InMemorySessionService>,
    secrets: Vec<ModelSecret>,
}

impl std::fmt::Debug for AdkHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdkHarness")
            .field("app_name", &self.app_name)
            .finish_non_exhaustive()
    }
}

impl AdkHarness {
    fn safe_error(&self, error: &impl std::fmt::Display) -> String {
        let mut message = error.to_string();
        for secret in &self.secrets {
            if !secret.0.is_empty() {
                message = message.replace(&secret.0, "[REDACTED]");
            }
        }
        message
    }

    /// Uses the selected catalog model with the same ADK runner as the default
    /// assistant. Native Google search is intentionally only attached to Gemini.
    pub fn from_selected_model(
        config: &AgentConfig,
        connection: &ModelConnection,
        model: &ModelInfo,
        secret: &ModelSecret,
    ) -> Result<Self, Error> {
        let llm: Arc<dyn Llm> = match connection.kind {
            ProviderKind::OpenRouter => Arc::new(
                adk_model::OpenRouterClient::new(
                    adk_model::OpenRouterConfig::new(&secret.0, &model.id)
                        .with_base_url(&connection.base_url)
                        .with_title("SovraHarness"),
                )
                .map_err(Error::ModelInit)?,
            ),
            ProviderKind::Ollama | ProviderKind::OpenAiCompatible => {
                let base = if connection.kind == ProviderKind::Ollama {
                    format!("{}/v1", connection.base_url.trim_end_matches('/'))
                } else {
                    connection.base_url.clone()
                };
                let api_key = if connection.authentication == Authentication::Bearer {
                    secret.0.clone()
                } else {
                    String::new()
                };
                let compatible = adk_model::OpenAICompatibleConfig::new(api_key, &model.id)
                    .with_provider_name(&connection.name)
                    .with_base_url(base)
                    .with_parallel_tool_calls(false);
                let mut headers = http::HeaderMap::new();
                if connection.authentication == Authentication::ApiKey {
                    let value = http::HeaderValue::from_str(&secret.0).map_err(|_| {
                        Error::ModelInit(adk_core::AdkError::model("Invalid API key header value"))
                    })?;
                    headers.insert(
                        http::HeaderName::from_bytes(connection.api_key_header.as_bytes())
                            .map_err(|_| {
                                Error::ModelInit(adk_core::AdkError::model(
                                    "Invalid API key header name",
                                ))
                            })?,
                        value,
                    );
                }
                for (name, value) in [
                    ("OpenAI-Organization", &connection.organization),
                    ("OpenAI-Project", &connection.project),
                ] {
                    if !value.is_empty() {
                        headers.insert(
                            http::HeaderName::from_bytes(name.as_bytes())
                                .expect("static header name"),
                            http::HeaderValue::from_str(value).map_err(|_| {
                                Error::ModelInit(adk_core::AdkError::model(
                                    "Invalid connection header value",
                                ))
                            })?,
                        );
                    }
                }
                Arc::new(
                    adk_model::OpenAICompatible::new(compatible)
                        .map_err(Error::ModelInit)?
                        .with_default_headers(headers)
                        .map_err(Error::ModelInit)?,
                )
            }
            ProviderKind::Anthropic => {
                let mut anthropic =
                    adk_model::anthropic::AnthropicConfig::new(&secret.0, &model.id)
                        .with_base_url(&connection.base_url);
                if connection.base_url.starts_with("http://") {
                    anthropic = anthropic.allow_insecure_http();
                }
                Arc::new(adk_model::AnthropicClient::new(anthropic).map_err(Error::ModelInit)?)
            }
        };
        let tools = if model.supports("tools") {
            UiToolset::all_tools()
        } else {
            Vec::new()
        };
        let mut harness = Self::with_llm_and_tools(config, llm, tools)?;
        if !secret.0.is_empty() {
            harness.secrets.push(secret.clone());
        }
        Ok(harness)
    }

    /// Builds an `AdkHarness` backed by Google's `GeminiModel`, `GoogleSearchTool`, and
    /// `adk-ui`'s native `UiToolset` (`ag_ui` / `a2ui` surface rendering tools).
    pub fn from_gemini_config(config: &AgentConfig) -> Result<Self, Error> {
        let api_key = config.api_key.as_deref().ok_or(Error::MissingApiKey)?;
        let model = GeminiModel::new(api_key, &config.model).map_err(Error::ModelInit)?;
        let mut tools: Vec<Arc<dyn Tool>> = vec![Arc::new(GoogleSearchTool::new())];
        tools.extend(UiToolset::all_tools());
        Self::with_llm_and_tools(config, Arc::new(model), tools)
    }

    /// Builds an `AdkHarness` with a custom [`Llm`] and tool list (used for tests and custom models).
    pub fn with_llm_and_tools(
        config: &AgentConfig,
        llm: Arc<dyn Llm>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<Self, Error> {
        let instruction = if tools.is_empty() {
            config.instruction.clone()
        } else {
            format!("{}\n\n{}", config.instruction, UI_AGENT_PROMPT_SHORT)
        };
        let mut builder = LlmAgentBuilder::new(&config.agent_name)
            .description("Sovra AI assistant with native AG-UI surfaces")
            .instruction(instruction)
            .model(llm);
        for tool in tools {
            builder = builder.tool(tool);
        }
        let agent = builder.build().map_err(Error::AgentBuild)?;
        let sessions = Arc::new(InMemorySessionService::new());
        let runner = Runner::builder()
            .app_name(&config.app_name)
            .agent(Arc::new(agent))
            .session_service(sessions.clone())
            .build()
            .map_err(Error::RunnerBuild)?;
        Ok(Self {
            app_name: config.app_name.clone(),
            runner: Arc::new(runner),
            sessions,
            secrets: config.api_key.iter().cloned().map(ModelSecret).collect(),
        })
    }

    /// Interrupts an in-flight execution for `session_id`, returning `true` if a run was active.
    pub fn interrupt(&self, session_id: &str) -> bool {
        self.runner.interrupt(session_id)
    }

    /// Runs a user prompt in `session_id` and streams resulting [`Event`]s to `events`.
    pub async fn run_prompt(&self, session_id: &str, prompt: &str, events: &Sender<Event>) {
        if let Err(err) = self.ensure_session(session_id).await {
            let _ = events
                .send(Event::AgentError {
                    session_id: session_id.to_owned(),
                    message: self.safe_error(&err),
                })
                .await;
            return;
        }
        self.stream_turn(session_id, prompt, events).await;
    }

    async fn ensure_session(&self, session_id: &str) -> Result<(), Error> {
        let get_req = GetRequest {
            app_name: self.app_name.clone(),
            user_id: DEFAULT_USER_ID.to_owned(),
            session_id: session_id.to_owned(),
            num_recent_events: None,
            after: None,
        };
        if self.sessions.get(get_req).await.is_ok() {
            return Ok(());
        }
        let create_req = CreateRequest {
            app_name: self.app_name.clone(),
            user_id: DEFAULT_USER_ID.to_owned(),
            session_id: Some(session_id.to_owned()),
            state: HashMap::new(),
        };
        self.sessions
            .create(create_req)
            .await
            .map(|_| ())
            .map_err(Error::Session)
    }

    async fn stream_turn(&self, session_id: &str, prompt: &str, events: &Sender<Event>) {
        let content = Content::new("user").with_text(prompt);
        let mut stream = match self
            .runner
            .run_str(DEFAULT_USER_ID, session_id, content)
            .await
        {
            Ok(stream) => stream,
            Err(err) => {
                let message = self.safe_error(&err);
                tracing::warn!(%session_id, %message, "ADK runner invocation failed");
                let _ = events
                    .send(Event::AgentError {
                        session_id: session_id.to_owned(),
                        message,
                    })
                    .await;
                return;
            }
        };

        while let Some(item) = stream.next().await {
            match item {
                Ok(adk_event) => {
                    if !forward_adk_event(session_id, &adk_event, events).await {
                        return;
                    }
                }
                Err(err) => {
                    let message = self.safe_error(&err);
                    tracing::warn!(%session_id, %message, "ADK stream error");
                    let _ = events
                        .send(Event::AgentError {
                            session_id: session_id.to_owned(),
                            message,
                        })
                        .await;
                    return;
                }
            }
        }
        let _ = events
            .send(Event::AgentCompleted {
                session_id: session_id.to_owned(),
            })
            .await;
    }
}

async fn forward_adk_event(
    session_id: &str,
    adk_event: &adk_core::Event,
    events: &Sender<Event>,
) -> bool {
    if let Some(widget) = extract_agui_surface(adk_event)
        && events
            .send(Event::AgUiSurface {
                session_id: session_id.to_owned(),
                widget,
            })
            .await
            .is_err()
    {
        return false;
    }
    if let Some(text) = extract_text(adk_event)
        && events
            .send(Event::AgentChunk {
                session_id: session_id.to_owned(),
                text,
            })
            .await
            .is_err()
    {
        return false;
    }
    true
}

fn extract_agui_surface(event: &adk_core::Event) -> Option<AgUiWidget> {
    let val = event.actions.state_delta.get(ACTIVE_SURFACE_STATE_KEY)?;
    let surface_id = val.get("id")?.as_str()?.to_owned();
    let version = val
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    Some(AgUiWidget::SurfaceCard {
        title: format!("AG-UI Surface · {surface_id}"),
        summary: format!("Updated structured AG-UI surface (revision v{version})."),
        surface_id,
    })
}

fn extract_text(event: &adk_core::Event) -> Option<String> {
    let content = event.llm_response.content.as_ref()?;
    let mut text = String::new();
    for part in &content.parts {
        if let Part::Text { text: chunk } = part {
            text.push_str(chunk);
        }
    }
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{LlmRequest, LlmResponse, LlmResponseStream};

    struct FakeLlm {
        reply: String,
    }

    #[async_trait::async_trait]
    impl Llm for FakeLlm {
        fn name(&self) -> &'static str {
            "fake-gemini"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let response = LlmResponse {
                content: Some(Content::new("model").with_text(&self.reply)),
                ..LlmResponse::default()
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
        }
    }

    #[test]
    fn missing_api_key_returns_error() {
        let cfg = AgentConfig::default();
        let err = AdkHarness::from_gemini_config(&cfg).unwrap_err();
        assert!(matches!(err, Error::MissingApiKey));
    }

    #[test]
    fn runs_prompt_with_adk_ui_toolset_and_streams_chunks_offline() {
        futures::executor::block_on(async {
            let cfg = AgentConfig::default();
            let fake_llm = Arc::new(FakeLlm {
                reply: "Hello from ADK-Rust + AG-UI!".to_owned(),
            });
            let tools = UiToolset::all_tools();
            assert!(!tools.is_empty(), "adk-ui toolset must provide AG-UI tools");
            let harness = AdkHarness::with_llm_and_tools(&cfg, fake_llm, tools).unwrap();
            let (tx, rx) = async_channel::unbounded();

            harness.run_prompt("session-1", "Hi there", &tx).await;

            assert_eq!(
                rx.recv().await.unwrap(),
                Event::AgentChunk {
                    session_id: "session-1".into(),
                    text: "Hello from ADK-Rust + AG-UI!".into(),
                }
            );
            assert_eq!(
                rx.recv().await.unwrap(),
                Event::AgentCompleted {
                    session_id: "session-1".into(),
                }
            );
        });
    }
}
