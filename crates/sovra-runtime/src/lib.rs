//! Async engine. Owns the tokio runtime on a dedicated background thread and talks to
//! the UI only through [`Client`] — two runtime-agnostic channels carrying
//! [`sovra_core::Command`] in and [`sovra_core::Event`] out.
//!
//! The UI never sees tokio, and this crate never sees the UI.

mod artifacts;
mod auth;
mod chat_history;
mod connections;
mod development;
mod private_storage;
mod remote_control;
mod remote_reference;
pub use development::{DevelopmentConfig, run_development_mcp};
pub use remote_control::{RemoteConfig, RemoteEndpointFactory, RemoteEndpointFuture};
pub use remote_reference::{RemoteReferenceClient, run_remote_reference};
mod file_index;
mod files;
mod github;
mod google;
mod groups;
mod mcp;
mod memory_capture;
mod memory_review;
mod memory_semantic;
mod model_priority;
mod models;
mod recalling;
mod remembering;
mod schedules;
mod skills;
mod terminal;
pub use mcp::{SAMPLE_SERVER_FLAG, run_sample_mcp_server};
pub use terminal::TerminalConfig;
mod teams;
pub use groups::{GroupServices, QrEncoder, TransportFactory, TransportFuture};
pub use models::ModelsConfig;
pub use schedules::{JsonScheduleStore, MemoryScheduleStore};

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::decision_client::fetch_systemone_answers;
use sovra_adk::{AdkHarness, SearchClient};
use sovra_core::{
    AgentConfig, Command, Event, MemoryError, MemoryStore, RoutingConfig, SpecialistAgentSpec,
    Timestamp, default_sovra_srx_agents, evaluate_routing,
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
    #[error("development MCP failed: {0}")]
    Development(String),
    #[error("remote reference client failed: {0}")]
    RemoteReference(String),
}

/// The UI's handle to the runtime. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Client {
    commands: Sender<Command>,
    events: Receiver<Event>,
    completion: Option<Receiver<()>>,
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

    /// The composition root calls this once, after the UI event loop has exited.
    /// Waits for worker draining and executor teardown without competing with the
    /// UI for `Event::Stopped`. Never call it on an active UI or async thread.
    pub fn shutdown_blocking(&self) -> Result<(), Error> {
        let completion = self.completion.as_ref().ok_or(Error::Stopped)?;
        // The UI quit hook may already have closed the command receiver.
        let _ = self.send(Command::Shutdown);
        completion.recv_blocking().map_err(|_| Error::Stopped)
    }

    /// A client wired to in-memory channels instead of a runtime, so UI tests can
    /// script events and observe commands deterministically.
    pub fn loopback() -> (Self, Receiver<Command>, Sender<Event>) {
        let (commands, command_rx) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        (
            Self {
                commands,
                events,
                completion: None,
            },
            command_rx,
            event_tx,
        )
    }
}

/// A harness whose AI is a free computer in the group, when this computer's owner
/// turned on "Use a free computer's AI when this one has none".
async fn group_harness(groups: &groups::GroupRuntime, config: &AgentConfig) -> Option<AdkHarness> {
    let peer = groups.work_peer().await?;
    AdkHarness::through_group(config, peer)
        .map_err(|error| tracing::warn!(%error, "group helper unavailable"))
        .ok()
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
    start_with_channels(
        version,
        agent_config,
        memory,
        models,
        files,
        skills,
        index,
        None,
    )
}

/// Starts with a personal messaging link (`WhatsApp`) supplied by the composition root.
#[expect(
    clippy::too_many_arguments,
    reason = "the composition root hands over every adapter in one call"
)]
pub fn start_with_channels(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
    channel: Option<Arc<dyn sovra_core::connections::MessagingChannel>>,
) -> Result<Client, Error> {
    start_with_messaging(
        version,
        agent_config,
        memory,
        models,
        files,
        skills,
        index,
        MessagingChannels {
            whatsapp: channel,
            telegram: None,
        },
    )
}

/// The personal messaging adapters the composition root supplies.
#[derive(Default)]
pub struct MessagingChannels {
    pub whatsapp: Option<Arc<dyn sovra_core::connections::MessagingChannel>>,
    pub telegram: Option<Arc<dyn sovra_core::connections::MessagingChannel>>,
}

impl std::fmt::Debug for MessagingChannels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagingChannels")
            .field("whatsapp", &self.whatsapp.is_some())
            .field("telegram", &self.telegram.is_some())
            .finish()
    }
}

/// Starts with every personal messaging link (`WhatsApp`, Telegram) the composition root
/// supplies.
#[expect(
    clippy::too_many_arguments,
    reason = "the composition root hands over every adapter in one call"
)]
pub fn start_with_messaging(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
    messaging: MessagingChannels,
) -> Result<Client, Error> {
    start_with_auth(
        version,
        agent_config,
        memory,
        models,
        files,
        skills,
        index,
        messaging,
        None,
    )
}

/// Starts with Google Sign-In configured by the composition root, without development
/// control. The native app adds its explicit configuration through `start_with_services`.
#[expect(
    clippy::too_many_arguments,
    reason = "the composition root hands over every adapter in one call"
)]
pub fn start_with_auth(
    version: &str,
    agent_config: AgentConfig,
    memory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    files: Option<Arc<dyn sovra_core::files::FileStore>>,
    skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
    messaging: MessagingChannels,
    google: Option<sovra_providers::GoogleOAuthConfig>,
) -> Result<Client, Error> {
    start_with_services(
        version,
        agent_config,
        RuntimeServices {
            memory_extraction_slots: 0,
            memory,
            models,
            files,
            skills,
            index,
            messaging,
            signin: google,
            google: None,
            github: None,
            development: None,
            remote_control: None,
            probes: Vec::new(),
            terminal: None,
            mcp_catalogue: None,
            mcp_sample_program: None,
            schedules: None,
            groups: None,
            schedule_search: None,
        },
    )
}

/// Every adapter the composition root can hand the runtime; each is optional.
#[derive(Default)]
pub struct RuntimeServices {
    /// Background extraction slots, 1–16; zero selects the default of four.
    pub memory_extraction_slots: usize,
    pub memory: Option<MemoryFactory>,
    pub models: Option<ModelsConfig>,
    pub files: Option<Arc<dyn sovra_core::files::FileStore>>,
    pub skills: Option<Arc<dyn sovra_core::skills::SkillStore>>,
    pub index: Option<Arc<dyn sovra_core::file_index::FileIndex>>,
    pub messaging: MessagingChannels,
    /// Google Sign-In for the login screen.
    pub signin: Option<sovra_providers::GoogleOAuthConfig>,
    /// The person's Google account (Connections page and agent tools).
    pub google: Option<Arc<dyn sovra_core::google::GoogleConnector>>,
    /// The person's GitHub account (Connections page and agent tools).
    pub github: Option<Arc<dyn sovra_core::github::GitHubConnector>>,
    /// Local control follows the build startup policy and remains revocable in Settings.
    pub development: Option<DevelopmentConfig>,
    /// Independently paired phones; always starts disabled.
    pub remote_control: Option<RemoteConfig>,
    /// Extra catalogue probes. Saved messaging links are probed as well.
    pub probes: Vec<Arc<dyn sovra_core::connection_catalogue::LinkProbe>>,
    /// Terminal access for the assistant; `None` leaves the terminal tool out.
    pub terminal: Option<TerminalConfig>,
    /// The saved list of connected services (MCP servers). `None` keeps it in
    /// the credential store, as older builds did.
    pub mcp_catalogue: Option<std::path::PathBuf>,
    /// Sovra's own executable, for the built-in sample MCP server (started with
    /// [`SAMPLE_SERVER_FLAG`]). `None` hides the sample.
    pub mcp_sample_program: Option<std::path::PathBuf>,
    /// Recurring prompts. `None` keeps an in-memory book for the session.
    pub schedules: Option<Arc<dyn sovra_core::ScheduleStore>>,
    /// Groups of computers sharing helpers and knowledge. `None` turns groups off.
    pub groups: Option<GroupServices>,
    /// Search used when a saved research job comes due. `None` uses the same
    /// built-in client as chat, so a test can stub this without replacing chat search.
    pub schedule_search: Option<SearchClient>,
}

impl std::fmt::Debug for RuntimeServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeServices")
            .field("messaging", &self.messaging)
            .field("google", &self.google)
            .field("github", &self.github)
            .finish_non_exhaustive()
    }
}

/// Starts the runtime on its own thread with the adapters in `services`.
pub fn start_with_services(
    version: &str,
    agent_config: AgentConfig,
    services: RuntimeServices,
) -> Result<Client, Error> {
    let (mut client, commands, events) = Client::loopback();
    let commands_tx = client.commands.clone();
    let (stopped, completion) = async_channel::bounded(1);
    client.completion = Some(completion);
    // Civil-time offset must be read before this thread starts tokio workers;
    // `time::UtcOffset::current_local_offset` can fail once other threads exist.
    let tz_offset_minutes = local_offset_minutes();
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
                services,
                commands_tx,
                handle,
                commands,
                events.clone(),
                tz_offset_minutes,
            ));
            // Adapters may schedule asynchronous cleanup when their final handle
            // drops. Keep the executor alive for that work before cancelling it.
            runtime.block_on(drain_adapter_tasks(runtime.metrics()));
            drop(runtime);
            let _ = events.send_blocking(Event::Stopped);
            let _ = stopped.send_blocking(());
        })
        .map_err(Error::Start)?;
    Ok(client)
}

async fn drain_adapter_tasks(metrics: tokio::runtime::RuntimeMetrics) {
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while metrics.num_alive_tasks() != 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await;
    if drained.is_err() {
        tracing::warn!(
            remaining_tasks = metrics.num_alive_tasks(),
            "adapter cleanup exceeded shutdown deadline"
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the runtime thread owns adapters, channels, and the local civil-time offset"
)]
async fn serve(
    version: String,
    agent_config: AgentConfig,
    services: RuntimeServices,
    // Lets the messaging links run prompts through the same path as the UI.
    commands_tx: Sender<Command>,
    handle: tokio::runtime::Handle,
    commands: Receiver<Command>,
    ui_events: Sender<Event>,
    tz_offset_minutes: i32,
) {
    let RuntimeServices {
        memory: memory_factory,
        memory_extraction_slots,
        models,
        files,
        skills: skill_store,
        index,
        messaging,
        signin,
        google,
        github,
        development,
        remote_control,
        probes,
        terminal,
        mcp_catalogue,
        mcp_sample_program,
        schedules: schedule_store,
        groups: group_services,
        schedule_search: schedule_search_override,
    } = services;
    let channels = connections::Channels {
        whatsapp: messaging.whatsapp,
        telegram: messaging.telegram,
    };
    let known_groups = Arc::new(tokio::sync::RwLock::new(Vec::<
        sovra_core::connections::SyncedGroup,
    >::new()));
    // Every event passes the connections task first: answers for WhatsApp and Telegram
    // sessions go back to the phone instead of the chat window.
    let connections = connections::ConnectionsRuntime::start(
        channels,
        ui_events.clone(),
        commands_tx.clone(),
        &handle,
        probes,
        known_groups.clone(),
    );
    let events = connections.route_events(ui_events, &handle);
    let mut auth = auth::AuthRuntime::start(
        signin,
        google.clone(),
        models.as_ref().map(|config| config.repository.clone()),
        events.clone(),
        &handle,
    );
    let google = google::GoogleRuntime::start(google, events.clone(), &handle);
    let github = github::GitHubRuntime::start(github, events.clone(), &handle);
    let groups = groups::GroupRuntime::start(group_services, events.clone(), &handle);
    let terminal = terminal::TerminalRuntime::start(terminal, events.clone(), &handle).await;
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
    let memory_control = remembering::MemoryControl::new();
    let memory_repository = models.as_ref().map(|config| config.repository.clone());
    let mcp_repository = models.as_ref().map(|config| config.repository.clone());
    if models.is_none() {
        memory_control.publish(true, harness.clone());
    }
    let models = models.map(|config| {
        models::ModelsRuntime::start(
            config,
            agent_config.clone(),
            events.clone(),
            memory_control.clone(),
        )
    });
    let mut mcp = mcp::McpRuntime::start(
        mcp_repository,
        mcp_catalogue,
        mcp_sample_program,
        terminal.environment(),
        events.clone(),
        &handle,
    );
    let mut runs = std::collections::BTreeMap::<String, ActiveRun>::new();
    let mut memory_taps = tokio::task::JoinSet::new();
    let mut routing_config = RoutingConfig::default();
    let mut specialists = default_sovra_srx_agents();
    let memory = open_memory(memory_factory).await;
    auth.attach_store(memory.clone());
    {
        // Federated recall answers from this computer's own memory, by keyword, and
        // only while remembering is on (the reader is `None` when it is paused).
        let store = memory.clone();
        let control = memory_control.clone();
        groups.attach_memory(Arc::new(move || {
            recalling::reader(store.as_ref(), &control, None)
        }));
    }
    {
        // Work handoff answers group members with whichever AI is selected now.
        let execution = models.as_ref().map(|models| models.execution.clone());
        let legacy = harness.clone();
        groups.attach_model_unless_given(Arc::new(move || {
            let execution = execution.clone();
            let legacy = legacy.clone();
            Box::pin(async move {
                let selected = match execution {
                    Some(execution) => match execution.read().await.clone() {
                        models::ModelExecution::Selected { harness, .. } => Some(*harness),
                        models::ModelExecution::Legacy => legacy,
                        models::ModelExecution::Unavailable(_) => None,
                    },
                    None => legacy,
                };
                selected.map(|harness| harness.work_model())
            })
        }));
    }
    let mut memory_semantic = memory_semantic::SemanticRuntime::start(
        memory.clone(),
        memory_repository,
        memory_control.clone(),
        events.clone(),
        &handle,
    );
    let mut memory_review = memory_review::MemoryReviewRuntime::start(
        memory.clone(),
        memory_control.clone(),
        memory_semantic.service.clone(),
        events.clone(),
        &handle,
    );
    // Browser views are throwaway copies, so the OS temp directory is the right home.
    let artifact_library = artifacts::ArtifactRuntime::new(
        memory.clone(),
        events.clone(),
        std::env::temp_dir().join("sovra-artifact-views"),
        terminal.project_access(),
    );
    let team = teams::TeamRuntime::new(memory.clone(), events.clone());
    connections.attach_store(memory.clone());
    tracing::info!(
        %version,
        adk_configured = harness.is_some(),
        memory_configured = memory.is_some(),
        "runtime started"
    );
    if events.send(Event::Ready { version }).await.is_err() {
        return;
    }
    let mut remembering = memory.as_ref().map(|store| {
        remembering::RememberingRuntime::start(
            store.clone(),
            memory_control.clone(),
            events.clone(),
            &handle,
            memory_extraction_slots,
        )
    });
    if load_skills {
        skills.send(sovra_core::skills::SkillCommand::Refresh);
    }
    let mut chat_history =
        chat_history::ChatHistoryRuntime::start(memory.clone(), events.clone(), &handle);
    if memory.is_some() {
        chat_history.load();
    }
    let remote_control =
        remote_control::RemoteRuntime::start(remote_control, events.clone(), &handle);
    let development = development
        .map(|config| development::DevelopmentRuntime::start(config, events.clone(), &handle));
    google.send(sovra_core::google::GoogleCommand::Load);
    github.send(sovra_core::github::GitHubCommand::Load);
    let (schedules, events) = schedules::SchedulesRuntime::start(
        schedule_store,
        events,
        commands_tx,
        tz_offset_minutes,
        &handle,
        known_groups,
        schedule_search_override.unwrap_or_else(|| search.clone()),
    );
    while let Ok(command) = commands.recv().await {
        tracing::debug!("command received");
        // Protect setup and capture too: immediately dispatched extraction must
        // not race ahead of the foreground guard owned by the spawned turn.
        let _interactive_dispatch = matches!(
            &command,
            Command::RunPrompt { .. }
                | Command::RunWithFiles { .. }
                | Command::Teams(sovra_core::teams::TeamCommand::Run { .. })
        )
        .then(model_priority::interactive);
        let captured_input = if let Some(worker) = &remembering {
            match &command {
                Command::RunPrompt {
                    session_id,
                    prompt,
                    context,
                }
                | Command::RunWithFiles {
                    session_id,
                    prompt,
                    context,
                    ..
                } if connections.private_access(session_id) => {
                    worker
                        .record_user(
                            session_id,
                            prompt,
                            context
                                .as_ref()
                                .and_then(sovra_core::ConversationContext::current),
                        )
                        .await
                }
                Command::Teams(sovra_core::teams::TeamCommand::Run {
                    session_id,
                    prompt,
                    context,
                }) if connections.private_access(session_id) => {
                    worker
                        .record_user(session_id, prompt, context.current())
                        .await
                }
                _ => None,
            }
        } else {
            None
        };
        runs.retain(|_, run| !run.task.is_finished());
        while memory_taps.try_join_next().is_some() {}
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
            Command::RunPrompt {
                session_id,
                prompt,
                context,
            } if session_files.contains_key(&session_id)
                && !(web_search_enabled && skills::web_without_files(&skill_catalog, &prompt)) =>
            {
                let attachment_ids = session_files.get(&session_id).cloned().unwrap_or_default();
                Command::RunWithFiles {
                    context,
                    session_id,
                    prompt,
                    attachment_ids,
                }
            }
            Command::RunWithFiles {
                context,
                session_id,
                prompt,
                attachment_ids,
            } if web_search_enabled && skills::web_without_files(&skill_catalog, &prompt) => {
                session_files.insert(session_id.clone(), attachment_ids);
                Command::RunPrompt {
                    session_id,
                    prompt,
                    context,
                }
            }
            other => other,
        };
        match command {
            Command::Auth(command) => auth.handle(command),
            Command::Memory(command) => memory_review.send(command),
            Command::Skills(command) => skills.send(command),
            Command::Google(command) => google.send(command),
            Command::GitHub(command) => github.send(command),
            Command::Schedule(command) => schedules.send(command),
            Command::Group(command) => {
                if let Err(problem) = groups.send(command) {
                    let _ = events
                        .send(Event::Group(Box::new(
                            sovra_core::group::GroupEvent::Problem(problem),
                        )))
                        .await;
                }
            }
            Command::FileIndex(command) => file_index.send(command),
            Command::Teams(sovra_core::teams::TeamCommand::Run {
                session_id,
                prompt,
                context,
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
                let capture = memory_capture::Capture::new(Some(&context), captured_input);
                let inputs = capture.inputs.clone();
                let base = match base
                    .with_reply_input_observer(inputs.clone())
                    .and_then(|base| {
                        base.with_conversation_memory(
                            inputs.reader(groups.knowledge.wrap(recalling::reader(
                                memory.as_ref(),
                                &memory_control,
                                memory_semantic.service.clone(),
                            ))),
                            &prompt,
                            now(),
                        )
                    })
                    .and_then(|base| base.with_indexed_files(file_index.reader.clone()))
                {
                    Ok(base) => base,
                    Err(error) => {
                        let _ = events
                            .send(Event::AgUi {
                                session_id,
                                event: sovra_core::AgUiEvent::RunError {
                                    message: error.to_string(),
                                },
                            })
                            .await;
                        continue;
                    }
                };
                let google_tools = google.tools().await;
                let base = base.with_google(google_tools.as_ref()).unwrap_or(base);
                let github_tools = github.tools().await;
                let base = base.with_github(github_tools.as_ref()).unwrap_or(base);
                let schedule_tools = schedules.tools();
                let base = base.with_schedules(Some(&schedule_tools)).unwrap_or(base);
                if let Some(previous) = runs.remove(&session_id) {
                    previous.interrupt(&session_id);
                    previous.task.abort();
                }
                let output = if connections.private_access(&session_id) && memory_control.enabled()
                {
                    memory.as_ref().map_or_else(
                        || events.clone(),
                        |store| {
                            memory_capture::tap(
                                &mut memory_taps,
                                store.clone(),
                                events.clone(),
                                memory_control.clone(),
                                capture,
                            )
                        },
                    )
                } else {
                    events.clone()
                };
                let run = team.with_events(output).spawn_run(
                    &handle,
                    base,
                    session_id.clone(),
                    prompt,
                    context.team_history(),
                    timeout,
                );
                runs.insert(session_id, run);
            }
            Command::Teams(command) => team.handle(&handle, command),
            Command::Artifacts(command) => artifact_library.handle(&handle, command),
            Command::Connections(command) => connections.handle(command),
            Command::RemoteControl(command) => remote_control.send(command),
            Command::Development(command) => {
                if let Some(worker) = &development {
                    worker.send(command);
                }
            }
            Command::Mcp(command) => mcp.handle(command),
            Command::Terminal(command) => terminal.handle(command),
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
                context,
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
                let capture = memory_capture::Capture::new(context.as_ref(), captured_input);
                capture.inputs.untracked();
                let output = if connections.private_access(&session_id) && memory_control.enabled()
                {
                    memory.as_ref().map_or_else(
                        || events.clone(),
                        |store| {
                            memory_capture::tap(
                                &mut memory_taps,
                                store.clone(),
                                events.clone(),
                                memory_control.clone(),
                                capture,
                            )
                        },
                    )
                } else {
                    events.clone()
                };
                let run = files::spawn_prompt(
                    &handle,
                    files::FilePrompt {
                        messaging: connections.messaging(&session_id),
                        google: google.tools().await,
                        github: github.tools().await,
                        terminal: if connections.private_access(&session_id) {
                            terminal.access()
                        } else {
                            None
                        },
                        context,
                        session_id: session_id.clone(),
                        prompt,
                        ids: attachment_ids,
                        store: store.clone(),
                        skills: skill_catalog.clone(),
                        skill_resources: skills.resources.clone(),
                        indexed_files: file_index.reader.clone(),
                        memory_reader: groups.knowledge.wrap(recalling::reader(
                            memory.as_ref(),
                            &memory_control,
                            memory_semantic.service.clone(),
                        )),
                        models: models.file_models.clone(),
                        events: output,
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
                chat_history.load();
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
                chat_history.save(thread);
            }
            Command::DeleteChatThread { session_id } => {
                session_files.remove(&session_id);
                chat_history.delete(session_id);
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
            Command::RunPrompt {
                session_id,
                prompt,
                context,
            } => {
                let (selected, timeout) = if let Some(models) = &models {
                    match models.execution.read().await.clone() {
                        models::ModelExecution::Legacy => match harness.clone() {
                            Some(harness) => (Some(harness), None),
                            None => (group_harness(&groups, &agent_config).await, None),
                        },
                        models::ModelExecution::Selected { harness, timeout } => {
                            (Some(*harness), Some(timeout))
                        }
                        // No AI here: a free computer in the group may answer instead.
                        models::ModelExecution::Unavailable(_)
                            if let Some(helper) = group_harness(&groups, &agent_config).await =>
                        {
                            (Some(helper), None)
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
                    match harness.clone() {
                        Some(harness) => (Some(harness), None),
                        None => (group_harness(&groups, &agent_config).await, None),
                    }
                };
                let memory_enabled = models
                    .as_ref()
                    .is_none_or(|m| m.memory_enabled.load(std::sync::atomic::Ordering::Relaxed));
                let web_search_enabled = models.as_ref().is_none_or(|m| {
                    m.web_search_enabled
                        .load(std::sync::atomic::Ordering::Relaxed)
                });
                let capture = memory_capture::Capture::new(context.as_ref(), captured_input);
                let inputs = capture.inputs.clone();
                let events = if memory_enabled && connections.private_access(&session_id) {
                    memory.as_ref().map_or_else(
                        || events.clone(),
                        |store| {
                            memory_capture::tap(
                                &mut memory_taps,
                                store.clone(),
                                events.clone(),
                                memory_control.clone(),
                                capture,
                            )
                        },
                    )
                } else {
                    events.clone()
                };
                if let Some(previous) = runs.remove(&session_id) {
                    previous.interrupt(&session_id);
                    previous.task.abort();
                }
                let private_access = connections.private_access(&session_id);
                let google_tools = if private_access {
                    google.tools().await
                } else {
                    None
                };
                let github_tools = if private_access {
                    github.tools().await
                } else {
                    None
                };
                let selected = selected.map(|base| {
                    if !private_access {
                        return base;
                    }
                    let base = base
                        .with_artifact_saver(artifact_library.saver())
                        .unwrap_or(base);
                    let base = base.with_google(google_tools.as_ref()).unwrap_or(base);
                    let base = base.with_github(github_tools.as_ref()).unwrap_or(base);
                    let base = match terminal.access() {
                        Some(access) => base.with_terminal(access).unwrap_or(base),
                        None => base,
                    };
                    let schedule_tools = schedules.tools();
                    let base = base.with_schedules(Some(&schedule_tools)).unwrap_or(base);
                    base.with_reply_input_observer(inputs.clone())
                        .unwrap_or(base)
                });
                let run = spawn_prompt(
                    &handle,
                    PromptJob {
                        private_access,
                        context,
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
                        indexed_files: if private_access {
                            file_index.reader.clone()
                        } else {
                            None
                        },
                        messaging: connections.messaging(&session_id),
                        remote_tools: if private_access {
                            mcp.offers()
                        } else {
                            Vec::new()
                        },
                        remote_caller: mcp.caller(),
                        memory_reader: if private_access {
                            inputs.reader(groups.knowledge.wrap(recalling::reader(
                                memory.as_ref(),
                                &memory_control,
                                memory_semantic.service.clone(),
                            )))
                        } else {
                            None
                        },
                    },
                );
                runs.insert(session_id, run);
            }
            Command::InterruptSession { session_id } => {
                schedules.run_finished(session_id.clone(), false);
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
    artifact_library.stop().await;
    memory_taps.abort_all();
    while memory_taps.join_next().await.is_some() {}
    if let Some(worker) = &mut remembering {
        worker.stop().await;
    }
    memory_review.stop().await;
    memory_semantic.stop().await;
    chat_history.stop().await;
    remote_control.stop().await;
    if let Some(worker) = development {
        worker.stop().await;
    }
    mcp.stop().await;
    auth.stop().await;
    tracing::info!("runtime stopped");
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

fn local_offset_minutes() -> i32 {
    time::UtcOffset::current_local_offset().map_or(0, |offset| i32::from(offset.whole_minutes()))
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
    private_access: bool,
    context: Option<sovra_core::ConversationContext>,
    memory_reader: Option<Arc<dyn sovra_core::MemoryReader>>,
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
    messaging: Option<Arc<dyn sovra_core::communications::MessagingAccess>>,
    remote_tools: Vec<sovra_core::mcp::ToolOffer>,
    remote_caller: Arc<mcp::ServiceCaller>,
}

fn spawn_prompt(handle: &tokio::runtime::Handle, job: PromptJob) -> ActiveRun {
    let PromptJob {
        private_access,
        context,
        memory_reader,
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
        messaging,
        remote_tools,
        remote_caller,
    } = job;
    let slot = Arc::new(std::sync::RwLock::new(harness.clone()));
    let harness_slot = Arc::clone(&slot);
    let interactive = model_priority::interactive();
    let task = handle.spawn(async move {
        let _interactive = interactive;
        let (skill, prompt) = match if private_access { skills::resolve(&skills, &prompt) } else { Ok((None, prompt.clone())) } {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = events.send(Event::AgentError { session_id, message: error.to_string() }).await;
                return;
            }
        };
        let deep_research = skill.as_ref().is_some_and(|skill| {
            sovra_core::is_deep_research_skill(&skill.name)
                || sovra_core::skill_requests_deep_research(&skill.tools)
        });
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
        let base = if private_access { base } else {
            match base.for_untrusted_sender() {
                Ok(base) => base,
                Err(error) => {
                    let _ = events.send(Event::AgentError { session_id, message: error.to_string() }).await;
                    return;
                }
            }
        };
        // Every turn is the orchestrator. The setting only adds or removes
        // WebSearchAgent; it does not replace the parent with that specialist.
        // Deep research always needs web search for its parallel goal helpers.
        if deep_research && !web_search_enabled {
            let _ = events
                .send(Event::AgUi {
                    session_id,
                    event: sovra_core::AgUiEvent::RunError {
                        message: "Deep research needs web search switched on in Models.".into(),
                    },
                })
                .await;
            return;
        }
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
        let adk = match adk.with_working_context(context.as_ref()).and_then(|adk| adk.with_conversation_memory(memory_reader, &prompt, now())).and_then(|adk| adk.with_indexed_files(indexed_files))
            .and_then(|adk| adk.with_messaging(messaging)).and_then(|adk| if private_access { adk.with_skill_library(skills, skill_resources) } else { Ok(adk) })
            .and_then(|adk| {
                if deep_research {
                    // Dedicated parallel research path — skip the specialist procedure.
                    Ok(adk)
                } else {
                    skill.as_ref().map_or_else(|| Ok(adk.clone()), |skill| adk.with_skill(skill))
                }
            })
            .and_then(|adk| {
                if remote_tools.is_empty() {
                    Ok(adk)
                } else {
                    adk.with_remote_tools(&remote_tools, remote_caller)
                }
            }) {
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
        if deep_research {
            let turn = sovra_adk::DeepResearchTurn {
                session_id: session_id.clone(),
                run_id: sovra_adk::new_research_run_id(),
                prompt,
            };
            let run = adk.run_deep_research(&turn, &events);
            if let Some(timeout) = timeout {
                if tokio::time::timeout(timeout, run).await.is_err() {
                    adk.interrupt(&session_id);
                    let _ = events
                        .send(Event::AgUi {
                            session_id,
                            event: sovra_core::AgUiEvent::RunError {
                                message: "Deep research timed out. Increase the connection timeout or try a shorter question.".into(),
                            },
                        })
                        .await;
                }
            } else {
                run.await;
            }
            return;
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

    #[tokio::test]
    async fn auth_commands_are_dispatched_and_drained_before_shutdown() {
        let client = start("auth-test").unwrap();
        let events = client.events();
        assert!(matches!(events.recv().await.unwrap(), Event::Ready { .. }));
        client
            .send(Command::Auth(sovra_core::AuthCommand::Load))
            .unwrap();
        client.send(Command::Ping).unwrap();
        client.send(Command::Shutdown).unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut received = Vec::new();
            loop {
                let event = events.recv().await.unwrap();
                let stopped = event == Event::Stopped;
                received.push(event);
                if stopped {
                    return received;
                }
            }
        })
        .await
        .expect("sign-in work must drain without leaving a detached task alive");
        assert!(received.contains(&Event::Pong));
        assert!(
            received.contains(&Event::Auth(sovra_core::AuthEvent::Loaded {
                session: None,
                has_accounts: false,
                google_ready: false,
            }))
        );
    }

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
                context: None,
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
                context: None,
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
                context: None,
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
                context: None,
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
                context: None,
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
