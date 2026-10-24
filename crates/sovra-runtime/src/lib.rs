//! Async engine. Owns the tokio runtime on a dedicated background thread and talks to
//! the UI only through [`Client`] — two runtime-agnostic channels carrying
//! [`sovra_core::Command`] in and [`sovra_core::Event`] out.
//!
//! The UI never sees tokio, and this crate never sees the UI.

mod models;
pub use models::ModelsConfig;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use sovra_adk::AdkHarness;
use sovra_adk::decision_client::fetch_systemone_answers;
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
                memory,
                models,
                handle,
                commands,
                events,
            ));
        })
        .map_err(Error::Start)?;
    Ok(client)
}

async fn serve(
    version: String,
    agent_config: AgentConfig,
    memory_factory: Option<MemoryFactory>,
    models: Option<ModelsConfig>,
    handle: tokio::runtime::Handle,
    commands: Receiver<Command>,
    events: Sender<Event>,
) {
    let harness = build_harness(&agent_config);
    let models = models
        .map(|config| models::ModelsRuntime::start(config, agent_config.clone(), events.clone()));
    let mut runs = std::collections::BTreeMap::<String, ActiveRun>::new();
    let mut routing_config = RoutingConfig::default();
    let mut specialists = default_sovra_srx_agents();
    let memory = open_memory(memory_factory).await;
    tracing::info!(
        %version,
        adk_configured = harness.is_some(),
        memory_configured = memory.is_some(),
        "runtime started"
    );
    if events.send(Event::Ready { version }).await.is_err() {
        return;
    }
    if let Some(store) = &memory {
        spawn_load_chat_history(&handle, store.clone(), events.clone());
    }
    while let Ok(command) = commands.recv().await {
        tracing::debug!("command received");
        runs.retain(|_, run| !run.task.is_finished());
        match command {
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
                            (Some(harness), Some(timeout))
                        }
                        models::ModelExecution::Unavailable(message) => {
                            let _ = events
                                .send(Event::AgentError {
                                    session_id,
                                    message,
                                })
                                .await;
                            continue;
                        }
                    }
                } else {
                    (harness.clone(), None)
                };
                if let Some(store) = &memory {
                    record_episode(
                        &handle,
                        store.clone(),
                        &session_id,
                        EpisodeKind::UserMessage,
                        prompt.clone(),
                    );
                }
                let events = memory.as_ref().map_or_else(
                    || events.clone(),
                    |store| tap_agent_events(&handle, store.clone(), events.clone()),
                );
                if let Some(previous) = runs.remove(&session_id) {
                    previous.harness.interrupt(&session_id);
                    previous.task.abort();
                }
                if let Some(run) = dispatch_prompt(
                    selected,
                    &handle,
                    session_id.clone(),
                    prompt,
                    events,
                    timeout,
                )
                .await
                {
                    runs.insert(session_id, run);
                }
            }
            Command::InterruptSession { session_id } => {
                if let Some(run) = runs.remove(&session_id) {
                    run.harness.interrupt(&session_id);
                    run.task.abort();
                    tracing::info!(%session_id, "session interrupted");
                }
            }
            Command::Shutdown => break,
        }
    }
    for (session, run) in runs {
        run.harness.interrupt(&session);
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
                | Event::ChatHistoryLoaded { .. }
                | Event::Ready { .. }
                | Event::Pong
                | Event::RouteEvaluated { .. }
                | Event::AgentResponseStarted { .. }
                | Event::AgUiSurface { .. }
                | Event::AgentError { .. }
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
    harness: AdkHarness,
    task: tokio::task::JoinHandle<()>,
}

async fn dispatch_prompt(
    harness: Option<AdkHarness>,
    handle: &tokio::runtime::Handle,
    session_id: String,
    prompt: String,
    events: Sender<Event>,
    timeout: Option<std::time::Duration>,
) -> Option<ActiveRun> {
    let Some(adk) = harness else {
        let _ = events
            .send(Event::AgentError {
                session_id,
                message: "Gemini API key is not configured (set GEMINI_API_KEY or GOOGLE_API_KEY)"
                    .to_owned(),
            })
            .await;
        return None;
    };
    let harness = adk.clone();
    let task = handle.spawn(async move {
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
    Some(ActiveRun { harness, task })
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
}
