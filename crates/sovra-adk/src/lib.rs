//! ADK-Rust adapter crate for `SovraHarness`.
//!
//! Encapsulates `adk-core`, `adk-agent`, `adk-model`, `adk-runner`, `adk-session`,
//! `adk-tool`, and `adk-ui` (native AG-UI / A2UI surfaces) behind a pure `sovra-core`
//! boundary so upstream crates never depend on ADK types directly.

pub mod decision_client;
mod native_ui;

use std::collections::HashMap;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, EventTextDeltas, Llm, Part, Tool};
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{CreateRequest, GetRequest, InMemorySessionService, SessionService};
use adk_tool::GoogleSearchTool;
use async_channel::Sender;
use futures::StreamExt as _;
use sovra_core::models::{Authentication, ModelConnection, ModelInfo, ModelSecret, ProviderKind};
use sovra_core::{AgentConfig, Event, ResponseModel};

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
    response_model: ResponseModel,
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
            native_ui::tools()
        } else {
            Vec::new()
        };
        let mut harness = Self::with_llm_and_tools(config, llm, tools)?;
        harness.response_model = ResponseModel {
            model_id: model.id.clone(),
            name: model.name.clone(),
            provider: connection.name.clone(),
        };
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
        tools.extend(native_ui::tools());
        let mut harness = Self::with_llm_and_tools(config, Arc::new(model), tools)?;
        harness.response_model = ResponseModel {
            model_id: config.model.clone(),
            name: config.model.clone(),
            provider: "Google Gemini".into(),
        };
        Ok(harness)
    }

    /// Builds an `AdkHarness` with a custom [`Llm`] and tool list (used for tests and custom models).
    pub fn with_llm_and_tools(
        config: &AgentConfig,
        llm: Arc<dyn Llm>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Result<Self, Error> {
        let response_model = ResponseModel {
            model_id: llm.name().into(),
            name: llm.name().into(),
            provider: String::new(),
        };
        let instruction = if tools.is_empty() {
            config.instruction.clone()
        } else {
            format!(
                "{}\n\nNative UI tools are optional. Answer normal conversational questions directly in Markdown, including ordinary lists and tables. Use a native UI tool only when the user asks for an interactive interface or when a concrete choice or confirmation benefits from buttons. Do not turn greetings, explanations or simple answers into cards, menus or task workflows. Never invent progress, actions or tool results. Button replies contain surface_ref and a button_click event. Use only tools actually available in this session.",
                config.instruction
            )
        };
        let mut builder = LlmAgentBuilder::new(&config.agent_name)
            .description("Sovra conversational assistant")
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
            response_model,
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

        let mut deltas = EventTextDeltas::default();
        let mut response_model = Some(self.response_model.clone());
        let started = std::time::Instant::now();
        while let Some(item) = stream.next().await {
            match item {
                Ok(adk_event) => {
                    if !forward_adk_event(
                        session_id,
                        &deltas.push(&adk_event),
                        &mut response_model,
                        started,
                        events,
                    )
                    .await
                    {
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
    response_model: &mut Option<ResponseModel>,
    started: std::time::Instant,
    events: &Sender<Event>,
) -> bool {
    let widgets = native_ui::extract_surfaces(adk_event);
    let (thinking, text) = extract_output(adk_event);
    if (!widgets.is_empty() || text.is_some() || thinking.is_some())
        && let Some(model) = response_model.take()
        && events
            .send(Event::AgentResponseStarted {
                session_id: session_id.to_owned(),
                model,
            })
            .await
            .is_err()
    {
        return false;
    }
    if let Some(text) = thinking
        && events
            .send(Event::AgentThinkingChunk {
                session_id: session_id.to_owned(),
                text,
                elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            })
            .await
            .is_err()
    {
        return false;
    }
    for widget in widgets {
        if events
            .send(Event::AgUiSurface {
                session_id: session_id.to_owned(),
                widget,
            })
            .await
            .is_err()
        {
            return false;
        }
    }
    if let Some(text) = text
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

/// Splits model output into (reasoning, answer) text; either may be absent.
fn extract_output(event: &adk_core::Event) -> (Option<String>, Option<String>) {
    let Some(content) = event.llm_response.content.as_ref() else {
        return (None, None);
    };
    if !matches!(content.role.as_str(), "model" | "assistant") {
        return (None, None);
    }
    let mut thinking = String::new();
    let mut text = String::new();
    for part in &content.parts {
        match part {
            Part::Text { text: chunk } => text.push_str(chunk),
            Part::Thinking {
                thinking: chunk, ..
            } => thinking.push_str(chunk),
            _ => {}
        }
    }
    (
        (!thinking.is_empty()).then_some(thinking),
        (!text.is_empty()).then_some(text),
    )
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
            let tools = native_ui::tools();
            assert!(!tools.is_empty(), "adk-ui toolset must provide AG-UI tools");
            let harness = AdkHarness::with_llm_and_tools(&cfg, fake_llm, tools).unwrap();
            let (tx, rx) = async_channel::unbounded();

            harness.run_prompt("session-1", "Hi there", &tx).await;
            assert_eq!(
                rx.recv().await.unwrap(),
                Event::AgentResponseStarted {
                    session_id: "session-1".into(),
                    model: ResponseModel {
                        model_id: "fake-gemini".into(),
                        name: "fake-gemini".into(),
                        provider: String::new(),
                    },
                }
            );

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
    struct StreamingLlm;

    #[async_trait::async_trait]
    impl Llm for StreamingLlm {
        fn name(&self) -> &'static str {
            "streaming-test"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            assert!(stream, "normal chat must request incremental output");
            let response = |text: &str, partial| LlmResponse {
                content: Some(Content::new("model").with_text(text)),
                partial,
                provider_metadata: (!partial)
                    .then(|| serde_json::json!({"content_complete": true})),
                ..LlmResponse::default()
            };
            Ok(Box::pin(futures::stream::iter([
                Ok(response("Hello **", true)),
                Ok(response("世界", true)),
                Ok(response("Hello **世界**!", false)),
            ])))
        }
    }

    #[test]
    fn streaming_complete_snapshot_does_not_repeat_partial_markdown() {
        futures::executor::block_on(async {
            let harness = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                Arc::new(StreamingLlm),
                vec![],
            )
            .unwrap();
            let (tx, rx) = async_channel::unbounded();
            harness.run_prompt("stream", "Hello", &tx).await;
            assert!(
                matches!(rx.recv().await.unwrap(), Event::AgentResponseStarted { model, .. }
                if model.model_id == "streaming-test")
            );
            let mut chunks = Vec::new();
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::AgentChunk { text, .. } => chunks.push(text),
                    Event::AgentCompleted { .. } => {}
                    event => panic!("unexpected event: {event:?}"),
                }
            }
            assert_eq!(chunks, ["Hello **", "世界", "**!"]);
        });
    }

    struct ThinkingLlm;

    #[async_trait::async_trait]
    impl Llm for ThinkingLlm {
        fn name(&self) -> &'static str {
            "thinking-test"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let response = |thinking: &str, text: &str, partial| {
                let mut content = Content::new("model");
                if !thinking.is_empty() {
                    content.parts.push(Part::Thinking {
                        thinking: thinking.into(),
                        signature: None,
                    });
                }
                if !text.is_empty() {
                    content.parts.push(Part::Text { text: text.into() });
                }
                LlmResponse {
                    content: Some(content),
                    partial,
                    provider_metadata: (!partial)
                        .then(|| serde_json::json!({"content_complete": true})),
                    ..LlmResponse::default()
                }
            };
            Ok(Box::pin(futures::stream::iter([
                Ok(response("Let me ", "", true)),
                Ok(response("check.", "", true)),
                Ok(response("", "Answer", true)),
                Ok(response("Let me check.", "Answer", false)),
            ])))
        }
    }

    #[test]
    fn reasoning_streams_separately_from_the_answer_without_repeats() {
        futures::executor::block_on(async {
            let harness = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                Arc::new(ThinkingLlm),
                vec![],
            )
            .unwrap();
            let (tx, rx) = async_channel::unbounded();
            harness.run_prompt("think", "Hello", &tx).await;
            assert!(matches!(
                rx.recv().await.unwrap(),
                Event::AgentResponseStarted { .. }
            ));
            let (mut thinking, mut answer) = (String::new(), String::new());
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::AgentThinkingChunk { text, .. } => {
                        assert!(answer.is_empty(), "reasoning arrives before the answer");
                        thinking.push_str(&text);
                    }
                    Event::AgentChunk { text, .. } => answer.push_str(&text),
                    Event::AgentCompleted { .. } => {}
                    event => panic!("unexpected event: {event:?}"),
                }
            }
            assert_eq!(thinking, "Let me check.");
            assert_eq!(answer, "Answer");
        });
    }

    #[test]
    fn user_and_tool_text_are_not_assistant_output() {
        for role in ["user", "tool"] {
            let mut event = adk_core::Event::new("test");
            event.set_content(Content::new(role).with_text("not an assistant reply"));
            assert_eq!(extract_output(&event), (None, None));
            let mut model = Some(ResponseModel {
                model_id: "test".into(),
                name: "Test model".into(),
                provider: String::new(),
            });
            let (tx, rx) = async_channel::unbounded();
            assert!(futures::executor::block_on(forward_adk_event(
                "test",
                &event,
                &mut model,
                std::time::Instant::now(),
                &tx
            )));
            assert!(rx.is_empty());
            assert!(
                model.is_some(),
                "attribution waits for real assistant output"
            );
        }
    }
}
