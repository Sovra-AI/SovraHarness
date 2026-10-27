//! ADK-Rust adapter crate for `SovraHarness`.
//!
//! Encapsulates `adk-core`, `adk-agent`, `adk-model`, `adk-runner`, `adk-session`,
//! `adk-tool`, and `adk-ui` (native AG-UI / A2UI surfaces) behind a pure `sovra-core`
//! boundary so upstream crates never depend on ADK types directly.

mod admission;
mod artifacts;
pub mod decision_client;
mod file_index;
mod files;
mod memory;
mod native_ui;
mod orchestrator;
mod recall;
mod skills;
mod team;
mod web_search;
mod working_context;

pub use artifacts::ArtifactSaver;
pub use team::TeamTurn;
pub use web_search::SearchClient;

use std::collections::HashMap;
use std::sync::Arc;

use adk_core::{Agent, Content, EventTextDeltas, Llm, Part, Tool};
use adk_model::GeminiModel;
use adk_runner::Runner;
use adk_session::{CreateRequest, GetRequest, InMemorySessionService, SessionService};
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
    #[error("failed to build the web search client: {0}")]
    SearchClient(String),
    #[error("{0}")]
    Skill(String),
    #[error("{0}")]
    Memory(String),
}

/// High-level ADK-Rust harness managing sessions, AG-UI tools, and agent execution.
#[derive(Clone)]
pub struct AdkHarness {
    app_name: String,
    runner: Arc<Runner>,
    sessions: Arc<InMemorySessionService>,
    secrets: Vec<ModelSecret>,
    response_model: ResponseModel,
    llm: Arc<dyn Llm>,
    base_tools: Vec<Arc<dyn Tool>>,
    base_instruction: String,
    file_reader: Option<Arc<files::ReadAttachment>>,
    /// Built parent. Its sub-agents are the roster delegates.
    root: Arc<dyn Agent>,
    search: Option<SearchClient>,
    web_search_enabled: bool,
    tools_enabled: bool,
    memory_output: memory::OutputMode,
    /// Set by [`AdkHarness::with_artifact_saver`]; team members inherit it.
    artifact_saver: Option<ArtifactSaver>,
}

impl std::fmt::Debug for AdkHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdkHarness")
            .field("app_name", &self.app_name)
            .finish_non_exhaustive()
    }
}

impl AdkHarness {
    /// Attach only the scoped index reader supplied by the runtime. This never
    /// gives the model arbitrary path access or enables unsupported model tools.
    pub fn with_indexed_files(
        &self,
        reader: Option<Arc<dyn sovra_core::file_index::IndexedFiles>>,
    ) -> Result<Self, Error> {
        let mut tools = self.base_tools.clone();
        tools.retain(|tool| !file_index::TOOL_NAMES.contains(&tool.name()));
        if self.tools_enabled
            && let Some(reader) = reader
        {
            tools.extend(file_index::tools(&reader));
        }
        self.rebuild(self.base_instruction.clone(), tools)
    }
    /// Apply an explicitly selected procedure without inserting it into user history.
    pub fn with_skill(&self, skill: &sovra_core::skills::Skill) -> Result<Self, Error> {
        skill
            .validate()
            .map_err(|error| Error::Skill(error.to_string()))?;
        if !skill.enabled || !matches!(skill.kind, sovra_core::skills::SkillKind::Procedure) {
            return Err(Error::Skill(
                "This skill cannot run as a model procedure.".into(),
            ));
        }
        for tool in &skill.tools {
            if !self.skill_capabilities().contains(tool) {
                return Err(Error::Skill(format!(
                    "/{} needs {tool}, which is unavailable in this turn. Check tool settings and attach files if needed.",
                    skill.name
                )));
            }
        }
        self.rebuild(
            format!("{}\n\n{}", self.base_instruction, skill.instruction_block()),
            self.base_tools.clone(),
        )
    }

    /// A per-turn catalogue; replacement does not accumulate duplicate loaders.
    pub fn with_skill_catalog(
        &self,
        catalog: Arc<sovra_core::skills::SkillCatalog>,
    ) -> Result<Self, Error> {
        self.with_skill_library(catalog, None)
    }

    pub fn with_skill_library(
        &self,
        catalog: Arc<sovra_core::skills::SkillCatalog>,
        resources: Option<Arc<dyn sovra_core::skills::SkillResources>>,
    ) -> Result<Self, Error> {
        let mut tools = self.base_tools.clone();
        tools.retain(|tool| tool.name() != "load_skill");
        if self.tools_enabled {
            let mut loader = skills::LoadSkill::new(catalog, self.skill_capabilities());
            loader.resources = resources;
            tools.push(Arc::new(loader));
        }
        self.rebuild(self.base_instruction.clone(), tools)
    }

    fn skill_capabilities(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .base_tools
            .iter()
            .map(|tool| tool.name().into())
            .collect();
        names.extend(
            self.root
                .sub_agents()
                .iter()
                .map(|agent| agent.name().into()),
        );
        // `web_search` stays on WebSearchAgent. Advertising it here let `/web-search`
        // activate against a tool the parent strips.
        names
    }

    /// Roster delegates attached to the built orchestrator, in roster order.
    ///
    /// Each pair is the specialist id and description ADK exposes for that child.
    pub fn delegates(&self) -> Vec<(String, String)> {
        self.root
            .sub_agents()
            .iter()
            .map(|agent| (agent.name().to_owned(), agent.description().to_owned()))
            .collect()
    }

    /// Name of the agent that answers the turn.
    pub fn orchestrator_name(&self) -> &str {
        self.root.name()
    }

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
        let tools = web_search::chat_tools(model.supports("tools"), None);
        let llm = admission::wrap(
            llm,
            sovra_core::ContextBudget {
                context_tokens: model.context_length,
                max_output_tokens: model.max_output_tokens,
            },
        );
        let mut harness = Self::assemble(
            config,
            llm,
            tools,
            None,
            None,
            false,
            model.supports("tools"),
        )?;
        harness.response_model = ResponseModel {
            model_id: model.id.clone(),
            name: model.name.clone(),
            provider: connection.name.clone(),
        };
        harness.memory_output = memory::OutputMode::for_selected(connection.kind, model);
        if !secret.0.is_empty() {
            harness.secrets.push(secret.clone());
        }
        Ok(harness)
    }

    /// Builds an `AdkHarness` backed by Google's `GeminiModel` and `adk-ui`'s native
    /// `UiToolset` (`ag_ui` / `a2ui` surface rendering tools).
    ///
    /// Web search is not `GoogleSearchTool`. The orchestrator attaches the in-repo
    /// `SearXNG` tool to `WebSearchAgent` only when [`Self::with_web_search`] is used.
    pub fn from_gemini_config(config: &AgentConfig) -> Result<Self, Error> {
        let api_key = config.api_key.as_deref().ok_or(Error::MissingApiKey)?;
        let model = GeminiModel::new(api_key, &config.model).map_err(Error::ModelInit)?;
        let mut harness = Self::with_llm_and_tools(config, Arc::new(model), native_ui::tools())?;
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
        Self::assemble(
            config,
            admission::wrap(llm, sovra_core::ContextBudget::default()),
            tools,
            None,
            None,
            false,
            true,
        )
    }

    /// Same chat model and session. The parent stays the orchestrator and gains
    /// `WebSearchAgent` with the in-repo search tool.
    pub fn with_web_search(&self, client: SearchClient) -> Result<Self, Error> {
        self.rebuild_configured(
            self.base_instruction.clone(),
            self.tools_without_search(),
            Some(client),
            true,
        )
    }

    /// Drops `WebSearchAgent` so this turn cannot answer from web hits.
    pub fn without_web_search(&self) -> Result<Self, Error> {
        self.rebuild_configured(
            web_search::web_search_disabled_instruction(&self.base_instruction),
            self.tools_without_search(),
            None,
            false,
        )
    }

    /// Same provider session, specialized for the files explicitly attached to this chat.
    pub fn with_files(
        &self,
        files: Arc<Vec<sovra_core::files::FileContent>>,
        tools_enabled: bool,
    ) -> Result<Self, Error> {
        let reader = Arc::new(files::ReadAttachment::new(files));
        let mut tools = self.tools_without_search();
        tools.retain(|tool| tool.name() != "read_attachment");
        if tools_enabled {
            tools.push(reader.clone());
        }
        let mut base = self.clone();
        base.tools_enabled &= tools_enabled;
        let mut harness = base.rebuild(files::instruction(&self.base_instruction), tools)?;
        harness.file_reader = Some(reader);
        Ok(harness)
    }

    /// Executes actual text/media inputs. The caller has already validated the model's modalities.
    pub async fn run_with_files(
        &self,
        session_id: &str,
        prompt: &str,
        char_budget: usize,
        events: &Sender<Event>,
    ) {
        let Some(reader) = &self.file_reader else {
            let _ = events
                .send(Event::AgentError {
                    session_id: session_id.into(),
                    message: "No files are prepared for this reader. Add the files again.".into(),
                })
                .await;
            return;
        };
        if let Err(error) = self.ensure_session(session_id).await {
            let _ = events
                .send(Event::AgentError {
                    session_id: session_id.into(),
                    message: self.safe_error(&error),
                })
                .await;
            return;
        }
        self.stream_content(
            session_id,
            files::content(prompt, reader, char_budget),
            events,
        )
        .await;
    }

    fn tools_without_search(&self) -> Vec<Arc<dyn Tool>> {
        self.base_tools
            .iter()
            .filter(|tool| {
                let name = tool.name();
                name != "web_search" && name != "google_search"
            })
            .cloned()
            .collect()
    }

    fn rebuild(&self, instruction: String, tools: Vec<Arc<dyn Tool>>) -> Result<Self, Error> {
        self.rebuild_configured(
            instruction,
            tools,
            self.search.clone(),
            self.web_search_enabled,
        )
    }

    fn rebuild_configured(
        &self,
        instruction: String,
        tools: Vec<Arc<dyn Tool>>,
        search: Option<SearchClient>,
        web_search_enabled: bool,
    ) -> Result<Self, Error> {
        let config = AgentConfig {
            app_name: self.app_name.clone(),
            agent_name: orchestrator::ORCHESTRATOR_ID.to_owned(),
            model: self.response_model.model_id.clone(),
            instruction,
            api_key: None,
            ..AgentConfig::default()
        };
        let mut built = Self::assemble(
            &config,
            Arc::clone(&self.llm),
            tools,
            Some(Arc::clone(&self.sessions)),
            search,
            web_search_enabled,
            self.tools_enabled,
        )?;
        built.response_model.clone_from(&self.response_model);
        built.secrets.clone_from(&self.secrets);
        built.artifact_saver.clone_from(&self.artifact_saver);
        built.file_reader.clone_from(&self.file_reader);
        built.memory_output = self.memory_output;
        Ok(built)
    }

    fn assemble(
        config: &AgentConfig,
        llm: Arc<dyn Llm>,
        tools: Vec<Arc<dyn Tool>>,
        sessions: Option<Arc<InMemorySessionService>>,
        search: Option<SearchClient>,
        web_search_enabled: bool,
        tools_enabled: bool,
    ) -> Result<Self, Error> {
        let web_search_enabled = web_search_enabled && tools_enabled;
        let response_model = ResponseModel {
            model_id: llm.name().into(),
            name: llm.name().into(),
            provider: String::new(),
        };
        let base_instruction = config.instruction.clone();
        let instruction = if tools.is_empty() {
            base_instruction.clone()
        } else {
            format!(
                "{base_instruction}\n\nNative UI tools are optional. Answer normal conversational questions directly in Markdown, including ordinary lists and tables. Use a native UI tool only when the user asks for an interactive interface or when a concrete choice or confirmation benefits from buttons. Do not turn greetings, explanations or simple answers into cards, menus or task workflows. Never invent progress, actions or tool results. Button replies contain surface_ref and a button_click event. Use only tools actually available in this session."
            )
        };
        let stored_tools = tools
            .into_iter()
            .filter(|tool| {
                let name = tool.name();
                tools_enabled && name != "web_search" && name != "google_search"
            })
            .collect::<Vec<_>>();
        let root = orchestrator::build(orchestrator::OrchestratorParts {
            llm: Arc::clone(&llm),
            parent_tools: stored_tools.clone(),
            instruction,
            search: search.clone(),
            web_search_enabled,
            tools_enabled,
        })?;
        let sessions = sessions.unwrap_or_else(|| Arc::new(InMemorySessionService::new()));
        let session_service: Arc<dyn SessionService> =
            Arc::clone(&sessions) as Arc<dyn SessionService>;
        let runner = Runner::builder()
            .app_name(&config.app_name)
            .agent(Arc::clone(&root) as Arc<dyn Agent>)
            .session_service(session_service)
            .build()
            .map_err(Error::RunnerBuild)?;
        Ok(Self {
            app_name: config.app_name.clone(),
            runner: Arc::new(runner),
            sessions,
            response_model,
            secrets: config.api_key.iter().cloned().map(ModelSecret).collect(),
            llm,
            base_tools: stored_tools,
            base_instruction,
            file_reader: None,
            memory_output: memory::OutputMode::Prompt,
            artifact_saver: None,
            root,
            search,
            web_search_enabled,
            tools_enabled,
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
        self.stream_content(session_id, Content::new("user").with_text(prompt), events)
            .await;
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

    async fn stream_content(&self, session_id: &str, content: Content, events: &Sender<Event>) {
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
                    let message = if err.is_model() && err.is_unauthorized() {
                        rejected_key_message(&self.response_model.provider)
                    } else {
                        message
                    };
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

    /// Generates a short (3–6 word) conversation title using an isolated one-shot session
    /// on the active model, falling back to `derive_chat_title` if the model returns empty.
    pub async fn generate_title(
        &self,
        session_id: &str,
        prompt: &str,
        reply: &str,
        events: &Sender<Event>,
    ) {
        let title_session = format!("title-gen-{session_id}");
        if self.ensure_session(&title_session).await.is_err() {
            return;
        }
        let user_snippet: String = prompt.chars().take(240).collect();
        let reply_snippet: String = reply.chars().take(240).collect();
        let title_prompt = format!(
            "Write a concise, clear 3 to 6 word title summarizing this conversation. \
             Output ONLY the title words with no quotes, no markdown, and no trailing period.\n\n\
             User: {user_snippet}\nAssistant: {reply_snippet}"
        );
        let content = Content::new("user").with_text(&title_prompt);
        let Ok(mut stream) = self
            .runner
            .run_str(DEFAULT_USER_ID, &title_session, content)
            .await
        else {
            return;
        };
        let mut deltas = EventTextDeltas::default();
        let mut collected = String::new();
        while let Some(Ok(adk_event)) = stream.next().await {
            let delta_event = deltas.push(&adk_event);
            let (_, text) = extract_output(&delta_event);
            if let Some(chunk) = text {
                collected.push_str(&chunk);
            }
        }
        let cleaned = sovra_core::sanitize_generated_title(&collected);
        let final_title = if cleaned.is_empty() {
            sovra_core::derive_chat_title(prompt)
        } else {
            cleaned
        };
        if !final_title.is_empty() && final_title != sovra_core::NEW_CHAT_TITLE {
            let _ = events
                .send(Event::ChatTitleGenerated {
                    session_id: session_id.to_owned(),
                    title: final_title,
                })
                .await;
        }
    }
}

async fn forward_adk_event(
    session_id: &str,
    adk_event: &adk_core::Event,
    response_model: &mut Option<ResponseModel>,
    started: std::time::Instant,
    events: &Sender<Event>,
) -> bool {
    let mut widgets = native_ui::extract_surfaces(adk_event);
    widgets.extend(artifacts::saved_artifact_widgets(adk_event));
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

/// Says what to do instead of showing the raw `model.unauthorized` code: the reply
/// failed because the service refused the saved key, which only a new key fixes.
fn rejected_key_message(provider: &str) -> String {
    let provider = if provider.is_empty() {
        "Your AI service"
    } else {
        provider
    };
    format!(
        "{provider} didn't accept your key, so I couldn't answer. Open Models, choose Change key, and paste a fresh key."
    )
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
    struct RejectedKeyLlm;

    #[async_trait::async_trait]
    impl Llm for RejectedKeyLlm {
        fn name(&self) -> &'static str {
            "rejected-key"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            Err(adk_core::AdkError::unauthorized(
                adk_core::ErrorComponent::Model,
                "model.openrouter.unauthorized",
                "User not found.",
            ))
        }
    }

    /// Regression: a refused key surfaced as "Note: model.unauthorized: User not found."
    #[test]
    fn a_refused_key_says_how_to_fix_it() {
        futures::executor::block_on(async {
            let mut harness = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                Arc::new(RejectedKeyLlm),
                vec![],
            )
            .unwrap();
            harness.response_model.provider = "OpenRouter".into();
            let (tx, rx) = async_channel::unbounded();
            harness.run_prompt("refused", "Hi", &tx).await;
            let mut error = None;
            while let Ok(event) = rx.try_recv() {
                if let Event::AgentError { message, .. } = event {
                    error = Some(message);
                }
            }
            let error = error.expect("a refused key ends the turn with an error");
            assert!(
                error.starts_with("OpenRouter didn't accept your key"),
                "{error}"
            );
            assert!(!error.contains("unauthorized"), "{error}");
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
