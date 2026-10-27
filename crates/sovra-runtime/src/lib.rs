//! Async engine. Owns the tokio runtime on a dedicated background thread and talks to
//! the UI only through [`Client`] — two runtime-agnostic channels carrying
//! [`sovra_core::Command`] in and [`sovra_core::Event`] out.
//!
//! The UI never sees tokio, and this crate never sees the UI.

mod artifacts;
mod file_index;
mod files;
mod models;
mod skills;
mod teams;
pub use models::ModelsConfig;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::decision_client::fetch_systemone_answers;
use sovra_adk::{AdkHarness, SearchClient};
use sovra_core::{
    AgentConfig, Command, EpisodeKind, Event, MemoryError, MemoryStore, NewEpisode, RoutingConfig,
    SpecialistAgentSpec, Timestamp, default_sovra_srx_agents, evaluate_routing,
};

/// A deferred memory-store constructor. The composition root picks the backend; the
/// runtime awaits it on its own executor (stores need an async context to open).
pub type MemoryStoreFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<Arc<dyn MemoryStore>, MemoryError>> + Send>>;
/// Boxed so `sovra-app` can hand over any adapter without the runtime naming it.
pub type MemoryFactory = Box<dyn FnOnce() -> MemoryStoreFuture + Send>;

/// Errors surfaced to the composition root.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to start the async runtime")]
    Start(#[source] std::io::Error),
    #[error("the runtime has stopped")]
    Stopped,
}

/// The UI's handle to the runtime. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Client {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl Client {
    /// Queues a command. Never blocks; fails only once the runtime has stopped.
    pub fn send(&self, command: Command) -> Result<(), Error> {
        self.commands.try_send(command).map_err(|_| Error::Stopped)
    }

    /// The runtime's event stream. Await it on any executor (GPUI, tokio, tests).
    pub fn events(&self) -> Receiver<Event> {
        self.events.clone()
    }

    /// A client wired to in-memory channels instead of a runtime, so UI tests can
    /// script events and observe commands deterministically.
    pub fn loopback() -> (Self, Receiver<Command>, Sender<Event>) {
        let (commands, command_rx) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        (Self { commands, events }, command_rx, event_tx)
    }
}

/// Starts the runtime on its own thread with default [`AgentConfig`] and no persistent
/// memory, and returns the UI's handle.
pub fn start(version: &str) -> Result<Client, Error> {
    start_with_config(version, AgentConfig::default(), None)
}

/// Starts the runtime on its own thread with explicit [`AgentConfig`] and returns the UI's handle.
///
/// The runtime stops when it receives [`Command::Shutdown`] or every [`Client`] is dropped.
pub fn start_with_config(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
) -> Result<Client, Error> {
    start_with_models(version, agent_config, memory, None)
}

pub fn start_with_models(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
) -> Result<Client, Error> {
    start_with_files(version, agent_config, memory, models, None)
}

/// Starts the engine with a local attachment adapter supplied by the composition root.
pub fn start_with_files(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
) -> Result<Client, Error> {
    start_with_skills(version, agent_config, memory, models, files, None)
}

/// Starts with user-authored skill packages supplied by the composition root.
pub fn start_with_skills(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
) -> Result<Client, Error> {
    start_with_file_index(version, agent_config, memory, models, files, skills, None)
}

/// Starts with a persistent index of explicitly selected local folders.
pub fn start_with_file_index(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
) -> Result<Client, Error> {
    let (client, commands, events) = Client::loopback();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("sovra-runtime-worker")
        .build()
        .map_err(Error::Start)?;
    let version = version.to_owned();
    std::thread::Builder::new()
        .name("sovra-runtime".into())
        .spawn(move || {
            let handle = runtime.handle().clone();
            runtime.block_on(serve(
                version,
                agent_config,
                Services {
                    memory,
                    models,
                    files,
                    skills,
                    index,
                },
                handle,
                commands,
                events,
            ));
        })
        .map_err(Error::Start)?;
    Ok(client)
}

struct Services {
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
}

async fn serve(
    version: String,
    agent_config: AgentConfig,
    services: Services,
    handle: tokio::runtime::Handle,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let Services {
        memory: memory_factory,
        models,
        files,
        skills: skill_store,
        index,
    } = services;
    let mut file_index = file_index::IndexRuntime::start(
        index,
        events.clone(),
        &handle,
        models.as_ref().map(|m| m.repository.clone()),
    );
    let file_jobs = files
        .as_ref()
        .map(|store| files::FileRuntime::start(store.clone(), events.clone(), &handle));
    let load_skills = skill_store.is_some();
    let skills = skills::SkillsRuntime::start(skill_store, events.clone(), &handle);
    let mut session_files = std::collections::BTreeMap::<String, Vec<String>>::new();
    let harness = build_harness(&agent_config);
    let search =
        SearchClient::built_in(agent_config.searxng_base_url.as_deref()).unwrap_or_else(|error| {
            tracing::warn!(%error, "web search client unavailable");
            SearchClient::failing(error.to_string())
        });
    let models = models
        .map(|config| models::ModelsRuntime::start(config, agent_config.clone(), events.clone()));
    let mut runs = std::collections::BTreeMap::<String, ActiveRun>::new();
    let mut routing_config = RoutingConfig::default();
    let mut specialists = default_sovra_srx_agents();
    let memory = open_memory(memory_factory).await;
    // Browser views are throwaway copies, so the OS temp directory is the right home.
    let artifact_library = artifacts::ArtifactRuntime::new(
        memory.clone(),
        events.clone(),
        std::env::temp_dir().join("sovra-artifact-views"),
    );
    let team = teams::TeamRuntime::new(memory.clone(), events.clone());
    tracing::info!(
        %version,
        adk_configured = harness.is_some(),
        memory_configured = memory.is_some(),
        "runtime started"
    );
    if events.send(Event::Ready { version }).await.is_err() {
        return;
    }
    if load_skills {
        skills.send(sovra_core::skills::SkillCommand::Refresh);
    }
    if let Some(store) = &memory {
        spawn_load_chat_history(&handle, store.clone(), events.clone());
    }
    while let Ok(command) = commands.recv().await {
        tracing::debug!("command received");
        runs.retain(|_, run| !run.task.is_finished());
        let skill_catalog = skills.catalog.read().await.clone();
        // Files stay on the session. An explicit web-search question is still
        // answered by the orchestrator, which delegates to WebSearchAgent.
        // Loading those files on the search turn would hide that delegation.
        let web_search_enabled = models.as_ref().is_none_or(|runtime| {
            runtime
                .web_search_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
        });
        let command = match command {
            Command::RunPrompt { session_id, prompt }
                if session_files.contains_key(&session_id)
                    && !(web_search_enabled
                        && skills::web_without_files(&skill_catalog, &prompt)) =>
            {
                let attachment_ids = session_files.get(&session_id).cloned().unwrap_or_default();
                Command::RunWithFiles {
                    session_id,
                    prompt,
                    attachment_ids,
                }
            }
            Command::RunWithFiles {
                session_id,
                prompt,
                attachment_ids,
            } if web_search_enabled && skills::web_without_files(&skill_catalog, &prompt) => {
                session_files.insert(session_id.clone(), attachment_ids);
                Command::RunPrompt { session_id, prompt }
            }
            other => other,
        };
        match command {
            Command::Skills(command) => skills.send(command),
            Command::FileIndex(command) => file_index.send(command),
            Command::Teams(sovra_core::teams::TeamCommand::Run {
                session_id,
                prompt,
                history,
            }) => {
                let selected = match models.as_ref() {
                    Some(models) => match models.execution.read().await.clone() {
                        models::ModelExecution::Legacy => harness.clone().map(|h| (h, None)),
                        models::ModelExecution::Selected { harness, timeout } => {
                            Some((*harness, Some(timeout)))
                        }
                        models::ModelExecution::Unavailable(message) => {
                            let _ = events
                                .send(Event::AgUiSurface {
                                    session_id: session_id.clone(),
                                    widget: sovra_core::default_llm_setup_widget(Some(&message)),
                                })
                                .await;
                            None
                        }
                    },
                    None => harness.clone().map(|h| (h, None)),
                };
                let Some((base, timeout)) = selected else {
                    let _ = events
                        .send(Event::AgUi {
                            session_id,
                            event: sovra_core::AgUiEvent::RunError {
                                message: "Connect an AI in Models so your team can answer.".into(),
                            },
                        })
                        .await;
                    continue;
                };
                let base = base
                    .with_artifact_saver(artifact_library.saver())
                    .unwrap_or(base);
                if let Some(previous) = runs.remove(&session_id) {
                    previous.interrupt(&session_id);
                    previous.task.abort();
                }
                let run =
                    team.spawn_run(&handle, base, session_id.clone(), prompt, history, timeout);
                runs.insert(session_id, run);
            }
            Command::Teams(command) => team.handle(&handle, command),
            Command::Artifacts(command) => artifact_library.handle(&handle, command),
            Command::Files(command) => {
                if let Some(jobs) = &file_jobs {
                    jobs.send(command);
                } else if let sovra_core::files::FileCommand::Prepare { session_id, id, .. } =
                    command
                {
                    let _ = events
                        .send(Event::FilePrepared(sovra_core::files::FilePrepared {
                            session_id,
                            id,
                            result: Err(sovra_core::files::FileError(
                                "File reading is not available in this session.".into(),
                            )),
                        }))
                        .await;
                }
            }
            Command::RunWithFiles {
                session_id,
                prompt,
                attachment_ids,
            } => {
                let (Some(store), Some(models)) = (&files, &models) else {
                    let _ = events
                        .send(Event::AgentError {
                            session_id,
                            message: "Connect a model in Models to read your files.".into(),
                        })
                        .await;
                    continue;
                };
                if let Some(previous) = runs.remove(&session_id) {
                    previous.interrupt(&session_id);
                    previous.task.abort();
                }
                session_files.insert(session_id.clone(), attachment_ids.clone());
                let run = files::spawn_prompt(
                    &handle,
                    files::FilePrompt {
                        session_id: session_id.clone(),
                        prompt,
                        ids: attachment_ids,
                        store: store.clone(),
                        skills: skill_catalog.clone(),
                        skill_resources: skills.resources.clone(),
                        indexed_files: file_index.reader.clone(),
                        models: models.file_models.clone(),
                        events: events.clone(),
                        web_search_enabled,
                        search: search.clone(),
                    },
                );
                runs.insert(session_id, run);
            }
            Command::Models(command) => {
                if let Some(models) = &models {
                    models.send(command);
                } else {
                    let _ = events
                        .send(Event::Models(sovra_core::models::ModelEvent::Error(
                            "Model connections are unavailable in this runtime.".into(),
                        )))
                        .await;
                }
            }
            Command::LoadChatHistory => {
                if let Some(store) = &memory {
                    spawn_load_chat_history(&handle, store.clone(), events.clone());
                } else {
                    let _ = events
                        .send(Event::ChatHistoryLoaded {
                            threads: Vec::new(),
                        })
                        .await;
                }
            }
            Command::SaveChatThread { thread } => {
                if let Some(message) = thread
                    .messages
                    .iter()
                    .rev()
                    .find(|message| !message.attachments.is_empty())
                {
                    session_files.insert(
                        thread.id.clone(),
                        message
                            .attachments
                            .iter()
                            .map(|file| file.id.clone())
                            .collect(),
                    );
                }
                if let Some(store) = &memory {
                    let store = store.clone();
                    let mut thread = *thread;
                    if thread.updated_at.millis() <= 1_000_000 {
                        let current = now();
                        if thread.created_at.millis() <= 1_000_000 {
                            thread.created_at = current;
                        }
                        thread.updated_at = current;
                    }
                    drop(handle.spawn(async move {
                        if let Err(error) = store.save_chat_thread(&thread).await {
                            tracing::warn!(%error, "failed to save chat thread");
                        }
                    }));
                }
            }
            Command::DeleteChatThread { session_id } => {
                session_files.remove(&session_id);
                if let Some(store) = &memory {
                    let store = store.clone();
                    let at = now();
                    drop(handle.spawn(async move {
                        if let Err(error) = store.delete_chat_thread(&session_id, at).await {
                            tracing::warn!(%error, "failed to delete chat thread");
                        }
                    }));
                }
            }
            Command::UpdateRouting {
                mode,
                backend,
                global_model,
            } => {
                routing_config.mode = mode;
                routing_config.backend = backend;
                routing_config.global_model = global_model;
            }
            Command::SetSpecialistModel {
                specialist_id,
                model_override,
                preferred_tier,
            } => {
                if let Some(agent) = specialists.iter_mut().find(|a| a.id == specialist_id) {
                    agent.model_override = model_override;
                    agent.preferred_tier = preferred_tier;
                }
            }
            Command::SimulateRoute { prompt } => {
                spawn_route_simulation(
                    &handle,
                    routing_config.clone(),
                    specialists.clone(),
                    agent_config.api_key.clone(),
                    prompt,
                    events.clone(),
                );
            }
            Command::Ping => {
                if events.send(Event::Pong).await.is_err() {
                    return;
                }
            }
            Command::RunPrompt { session_id, prompt } => {
                let (selected, timeout) = if let Some(models) = &models {
                    match models.execution.read().await.clone() {
                        models::ModelExecution::Legacy => (harness.clone(), None),
                        models::ModelExecution::Selected { harness, timeout } => {
                            (Some(*harness), Some(timeout))
                        }
                        models::ModelExecution::Unavailable(message) => {
                            let _ = events
                                .send(Event::AgUiSurface {
                                    session_id: session_id.clone(),
                                    widget: sovra_core::default_llm_setup_widget(Some(&message)),
                                })
                                .await;
                            let _ = events.send(Event::AgentCompleted { session_id }).await;
                            continue;
                        }
                    }
                } else {
                    (harness.clone(), None)
                };
                let memory_enabled = models
                    .as_ref()
                    .is_none_or(|m| m.memory_enabled.load(std::sync::atomic::Ordering::Relaxed));
                let web_search_enabled = models.as_ref().is_none_or(|m| {
                    m.web_search_enabled
                        .load(std::sync::atomic::Ordering::Relaxed)
                });
                if memory_enabled && let Some(store) = &memory {
                    record_episode(
                        &handle,
                        store.clone(),
                        &session_id,
                        EpisodeKind::UserMessage,
                        prompt.clone(),
                    );
                }
                let events = if memory_enabled {
                    memory.as_ref().map_or_else(
                        || events.clone(),
                        |store| tap_agent_events(&handle, store.clone(), events.clone()),
                    )
                } else {
                    events.clone()
                };
                if let Some(previous) = runs.remove(&session_id) {
                    previous.interrupt(&session_id);
                    previous.task.abort();
                }
                // Any reply may keep what it makes in Artifacts (`save_artifact`).
                let selected = selected.map(|base| {
                    base.with_artifact_saver(artifact_library.saver())
                        .unwrap_or(base)
                });
                let run = spawn_prompt(
                    &handle,
                    PromptJob {
                        harness: selected,
                        session_id: session_id.clone(),
                        prompt,
                        events,
                        timeout,
                        routing: routing_config.clone(),
                        specialists: specialists.clone(),
                        api_key: agent_config.api_key.clone(),
                        web_search_enabled,
                        search: search.clone(),
                        skills: skill_catalog.clone(),
                        skill_resources: skills.resources.clone(),
                        indexed_files: file_index.reader.clone(),
                    },
                );
                runs.insert(session_id, run);
            }
            Command::InterruptSession { session_id } => {
                if let Some(run) = runs.remove(&session_id) {
                    run.interrupt(&session_id);
                    run.task.abort();
                    tracing::info!(%session_id, "session interrupted");
                }
            }
            Command::GenerateChatTitle {
                session_id,
                prompt,
                reply,
            } => {
                let selected = if let Some(models) = &models {
                    match models.execution.read().await.clone() {
                        models::ModelExecution::Legacy => harness.clone(),
                        models::ModelExecution::Selected { harness, .. } => Some(*harness),
                        models::ModelExecution::Unavailable(_) => None,
                    }
                } else {
                    harness.clone()
                };
                if let Some(adk) = selected {
                    let events = events.clone();
                    drop(handle.spawn(async move {
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(15),
                            adk.generate_title(&session_id, &prompt, &reply, &events),
                        )
                        .await;
                    }));
                } else {
                    let fallback = sovra_core::derive_chat_title(&prompt);
                    if !fallback.is_empty() && fallback != sovra_core::NEW_CHAT_TITLE {
                        let _ = events
                            .send(Event::ChatTitleGenerated {
                                session_id,
                                title: fallback,
                            })
                            .await;
                    }
                }
            }
            Command::Shutdown => break,
        }
    }
    for (session, run) in runs {
        run.interrupt(&session);
        run.task.abort();
    }
    let _ = events.send(Event::Stopped).await;
    tracing::info!("runtime stopped");
}

fn spawn_load_chat_history(
    handle: &tokio::runtime::Handle,
    store: Arc<dyn MemoryStore>,
    events: Sender<Event>,
) {
    drop(handle.spawn(async move {
        match store.list_chat_threads(50).await {
            Ok(threads) => {
                let _ = events.send(Event::ChatHistoryLoaded { threads }).await;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to load chat history");
            }
        }
    }));
}

fn spawn_route_simulation(
    handle: &tokio::runtime::Handle,
    routing: RoutingConfig,
    specialists: Vec<SpecialistAgentSpec>,
    api_key: Option<String>,
    prompt: String,
    events: Sender<Event>,
) {
    drop(handle.spawn(async move {
        let external = fetch_systemone_answers(&routing, api_key.as_deref(), &prompt)
            .await
            .ok()
            .flatten();
        let evaluation = evaluate_routing(
            &routing,
            &specialists,
            &prompt,
            external.as_ref(),
            routing.backend,
        );
        let _ = events
            .send(Event::RouteEvaluated {
                evaluation: Box::new(evaluation),
            })
            .await;
    }));
}

async fn open_memory(factory: Option<MemoryFactory>) -> Option<Arc<dyn MemoryStore>> {
    let store = match factory {
        Some(build) => match build().await {
            Ok(store) => store,
            Err(error) => {
                tracing::error!(%error, "failed to open the memory store; continuing without memory");
                return None;
            }
        },
        None => return None,
    };
    if let Err(error) = store.health().await {
        tracing::error!(%error, "memory store failed its health check; continuing without memory");
        return None;
    }
    Some(store)
}

fn record_episode(
    handle: &tokio::runtime::Handle,
    store: Arc<dyn MemoryStore>,
    session_id: &str,
    kind: EpisodeKind,
    content: String,
) {
    let episode = NewEpisode {
        session_id: session_id.to_owned(),
        kind,
        content,
        occurred_at: now(),
    };
    drop(handle.spawn(async move {
        if let Err(error) = store.record_episode(episode).await {
            tracing::warn!(%error, "failed to record episode");
        }
    }));
}

fn tap_agent_events(
    handle: &tokio::runtime::Handle,
    store: Arc<dyn MemoryStore>,
    events: Sender<Event>,
) -> Sender<Event> {
    let (tapped, stream) = async_channel::unbounded::<Event>();
    drop(handle.spawn(async move {
        let mut transcript = String::new();
        while let Ok(event) = stream.recv().await {
            match &event {
                Event::AgentChunk { text, .. } => transcript.push_str(text),
                Event::AgentCompleted { session_id } => {
                    if !transcript.is_empty() {
                        let episode = NewEpisode {
                            session_id: session_id.clone(),
                            kind: EpisodeKind::AgentMessage,
                            content: std::mem::take(&mut transcript),
                            occurred_at: now(),
                        };
                        if let Err(error) = store.record_episode(episode).await {
                            tracing::warn!(%error, "failed to record agent episode");
                        }
                    }
                }
                Event::Models(_)
                | Event::Skills(_)
                | Event::Teams(_)
                | Event::Artifacts(_)
                | Event::AgUi { .. }
                | Event::FilePrepared(_)
                | Event::FileIndex(_)
                | Event::ChatHistoryLoaded { .. }
                | Event::Ready { .. }
                | Event::Pong
                | Event::RouteEvaluated { .. }
                // Reasoning is the model's scratch work, not something to remember.
                | Event::AgentThinkingChunk { .. }
                | Event::AgentResponseStarted { .. }
                | Event::AgUiSurface { .. }
                | Event::AgentError { .. }
                | Event::ChatTitleGenerated { .. }
                | Event::Stopped => {}
            }
            if events.send(event).await.is_err() {
                return;
            }
        }
    }));
    tapped
}

fn now() -> Timestamp {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    Timestamp(millis)
}

fn build_harness(agent_config: &AgentConfig) -> Option<AdkHarness> {
    if !agent_config.has_api_key() {
        tracing::debug!("no Gemini API key configured; ADK harness waiting for credentials");
        return None;
    }
    match AdkHarness::from_gemini_config(agent_config) {
        Ok(harness) => Some(harness),
        Err(error) => {
            tracing::warn!(%error, "failed to initialize ADK Gemini harness");
            None
        }
    }
}

struct ActiveRun {
    harness: Arc<std::sync::RwLock<Option<AdkHarness>>>,
    task: tokio::task::JoinHandle<()>,
}

impl ActiveRun {
    fn interrupt(&self, session_id: &str) {
        let guard = self
            .harness
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(harness) = guard.as_ref() {
            harness.interrupt(session_id);
        }
    }
}

struct PromptJob {
    harness: Option<AdkHarness>,
    session_id: String,
    prompt: String,
    events: Sender<Event>,
    timeout: Option<std::time::Duration>,
    routing: RoutingConfig,
    specialists: Vec<SpecialistAgentSpec>,
    api_key: Option<String>,
    web_search_enabled: bool,
    search: SearchClient,
    skills: Arc<sovra_core::skills::SkillCatalog>,
    skill_resources: Option<Arc<dyn sovra_core::skills::SkillResources>>,
    indexed_files: Option<Arc<dyn sovra_core::file_index::IndexedFiles>>,
}

fn spawn_prompt(handle: &tokio::runtime::Handle, job: PromptJob) -> ActiveRun {
    let PromptJob {
        harness,
        session_id,
        prompt,
        events,
        timeout,
        routing,
        specialists,
        api_key,
        web_search_enabled,
        search,
        skills,
        skill_resources,
        indexed_files,
    } = job;
    let slot = Arc::new(std::sync::RwLock::new(harness.clone()));
    let harness_slot = Arc::clone(&slot);
    let task = handle.spawn(async move {
        let (skill, prompt) = match skills::resolve(&skills, &prompt) {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = events.send(Event::AgentError { session_id, message: error.to_string() }).await;
                return;
            }
        };
        let external = fetch_systemone_answers(&routing, api_key.as_deref(), &prompt)
            .await
            .ok()
            .flatten();
        let evaluation = evaluate_routing(
            &routing,
            &specialists,
            &prompt,
            external.as_ref(),
            routing.backend,
        );
        if events
            .send(Event::RouteEvaluated {
                evaluation: Box::new(evaluation),
            })
            .await
            .is_err()
        {
            return;
        }
        let Some(base) = harness else {
            let _ = events
                .send(Event::AgentError {
                    session_id,
                    message:
                        "Gemini API key is not configured (set GEMINI_API_KEY or GOOGLE_API_KEY)"
                            .to_owned(),
                })
                .await;
            return;
        };
        // Every turn is the orchestrator. The setting only adds or removes
        // WebSearchAgent; it does not replace the parent with that specialist.
        let adk = if web_search_enabled {
            match base.with_web_search(search) {
                Ok(adk) => adk,
                Err(error) => {
                    let _ = events
                        .send(Event::AgentError {
                            session_id,
                            message: error.to_string(),
                        })
                        .await;
                    return;
                }
            }
        } else {
            match base.without_web_search() {
                Ok(adk) => adk,
                Err(error) => {
                    let _ = events
                        .send(Event::AgentError {
                            session_id,
                            message: error.to_string(),
                        })
                        .await;
                    return;
                }
            }
        };
        let adk = match adk.with_indexed_files(indexed_files).and_then(|adk| adk.with_skill_library(skills, skill_resources))
            .and_then(|adk| skill.as_ref().map_or_else(|| Ok(adk.clone()), |skill| adk.with_skill(skill))) {
            Ok(adk) => adk,
            Err(error) => {
                let _ = events.send(Event::AgentError { session_id, message: error.to_string() }).await;
                return;
            }
        };
        {
            let mut guard = harness_slot
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard = Some(adk.clone());
        }
        let run = adk.run_prompt(&session_id, &prompt, &events);
        if let Some(timeout) = timeout {
            if tokio::time::timeout(timeout, run).await.is_err() {
                adk.interrupt(&session_id);
                let _ = events
                    .send(Event::AgentError {
                        session_id,
                        message: "The model response timed out. Increase the connection timeout or try again.".into(),
                    })
                    .await;
            }
        } else {
            run.await;
        }
    });
    ActiveRun {
        harness: slot,
        task,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_ping_and_shuts_down() {
        let client = start("9.9.9").unwrap();
        let events = client.events();

        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Ready {
                version: "9.9.9".into()
            }
        );
        client.send(Command::Ping).unwrap();
        assert_eq!(events.recv_blocking().unwrap(), Event::Pong);
        client.send(Command::Shutdown).unwrap();
        assert_eq!(events.recv_blocking().unwrap(), Event::Stopped);
    }

    #[test]
    fn unconfigured_prompt_emits_agent_error() {
        let client = start("9.9.9").unwrap();
        let events = client.events();

        assert_eq!(
            events.recv_blocking().unwrap(),
            Event::Ready {
                version: "9.9.9".into()
            }
        );
        client
            .send(Command::RunPrompt {
                session_id: "s-test".into(),
                prompt: "Hello".into(),
            })
            .unwrap();
        loop {
            match events.recv_blocking().unwrap() {
                Event::RouteEvaluated { .. } => {}
                Event::AgentError {
                    session_id,
                    message,
                } => {
                    assert_eq!(session_id, "s-test");
                    assert!(message.contains("GEMINI_API_KEY"));
                    break;
                }
                other => panic!("expected AgentError, got {other:?}"),
            }
        }
        client.send(Command::Shutdown).unwrap();
        assert_eq!(events.recv_blocking().unwrap(), Event::Stopped);
    }

    #[test]
    fn run_prompt_labels_explicit_web_search_before_the_missing_key_error() {
        let client = start("9.9.9").unwrap();
        let events = client.events();
        assert!(matches!(
            events.recv_blocking().unwrap(),
            Event::Ready { .. }
        ));

        client
            .send(Command::RunPrompt {
                session_id: "s-search".into(),
                prompt: "search the web for rust release notes".into(),
            })
            .unwrap();
        match events.recv_blocking().unwrap() {
            Event::RouteEvaluated { evaluation } => {
                assert_eq!(
                    evaluation.specialist_id.as_deref(),
                    Some(sovra_core::WEB_SEARCH_AGENT_ID)
                );
            }
            other => panic!("expected RouteEvaluated before the missing-key error, got {other:?}"),
        }
        match events.recv_blocking().unwrap() {
            Event::AgentError {
                session_id,
                message,
            } => {
                assert_eq!(session_id, "s-search");
                assert!(message.contains("GEMINI_API_KEY"));
            }
            other => panic!("expected missing-key error after the route, got {other:?}"),
        }

        client
            .send(Command::RunPrompt {
                session_id: "s-hello".into(),
                prompt: "hello".into(),
            })
            .unwrap();
        match events.recv_blocking().unwrap() {
            Event::RouteEvaluated { evaluation } => {
                assert_ne!(
                    evaluation.specialist_id.as_deref(),
                    Some(sovra_core::WEB_SEARCH_AGENT_ID)
                );
            }
            other => panic!("expected a route for the greeting, got {other:?}"),
        }
        match events.recv_blocking().unwrap() {
            Event::AgentError { session_id, .. } => assert_eq!(session_id, "s-hello"),
            other => panic!("expected missing-key error for the greeting, got {other:?}"),
        }

        client.send(Command::Shutdown).unwrap();
        assert_eq!(events.recv_blocking().unwrap(), Event::Stopped);
    }

    #[test]
    fn attached_files_do_not_hide_an_explicit_web_search() {
        let client = start("9.9.9").unwrap();
        let events = client.events();
        assert!(matches!(
            events.recv_blocking().unwrap(),
            Event::Ready { .. }
        ));

        client
            .send(Command::RunWithFiles {
                session_id: "s-files".into(),
                prompt: "search the web for rust release notes".into(),
                attachment_ids: vec!["cargo-deny-gather".into()],
            })
            .unwrap();
        match events.recv_blocking().unwrap() {
            Event::RouteEvaluated { evaluation } => {
                assert_eq!(
                    evaluation.specialist_id.as_deref(),
                    Some(sovra_core::WEB_SEARCH_AGENT_ID)
                );
            }
            other => panic!("expected WebSearchAgent before any file error, got {other:?}"),
        }
        match events.recv_blocking().unwrap() {
            Event::AgentError {
                session_id,
                message,
            } => {
                assert_eq!(session_id, "s-files");
                assert!(message.contains("GEMINI_API_KEY"), "{message}");
            }
            other => panic!("expected missing-key error after the route, got {other:?}"),
        }

        client
            .send(Command::RunPrompt {
                session_id: "s-files".into(),
                prompt: "what does this file say?".into(),
            })
            .unwrap();
        match events.recv_blocking().unwrap() {
            Event::AgentError {
                session_id,
                message,
            } => {
                assert_eq!(session_id, "s-files");
                assert!(
                    message.contains("Connect a model"),
                    "file question left the file path: {message}"
                );
            }
            other => panic!("expected the file reader path, got {other:?}"),
        }

        client.send(Command::Shutdown).unwrap();
        assert_eq!(events.recv_blocking().unwrap(), Event::Stopped);
    }
}
