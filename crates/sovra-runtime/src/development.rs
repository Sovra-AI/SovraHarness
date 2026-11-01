//! Development control actor and stdio MCP bridge. Owns all IO and cancellation.
mod endpoint;
mod server;

use async_channel::{Receiver, Sender};
use sovra_core::{
    Event,
    development::{DevelopmentCommand, DevelopmentEvent, DevelopmentState, UiRequest},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

pub use server::run_development_mcp;

pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const MAX_FRAME: usize = 32 * 1024;
pub(super) const MAX_RESPONSE: usize = 8 * 1024 * 1024;

/// Paths and executable are resolved once by the composition root.
#[derive(Debug, Clone)]
pub struct DevelopmentConfig {
    pub directory: PathBuf,
    pub executable: PathBuf,
}

pub(super) struct Incoming {
    generation: String,
    request: UiRequest,
    reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

struct Pending {
    deadline: Instant,
    reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

pub(crate) struct DevelopmentRuntime {
    commands: Sender<DevelopmentCommand>,
    task: JoinHandle<()>,
}

impl DevelopmentRuntime {
    pub(crate) fn start(
        config: DevelopmentConfig,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, receiver) = async_channel::unbounded();
        let task = handle.spawn(run(config, events, receiver));
        Self { commands, task }
    }

    pub(crate) fn send(&self, command: DevelopmentCommand) {
        if self.commands.try_send(command).is_err() {
            tracing::warn!("development control queue unavailable");
        }
    }

    pub(crate) async fn stop(self) {
        self.commands.close();
        let _ = self.task.await;
    }
}

async fn run(
    config: DevelopmentConfig,
    events: Sender<Event>,
    commands: Receiver<DevelopmentCommand>,
) {
    let mut state = DevelopmentState {
        connection_command: format!("\"{}\" --development-mcp", config.executable.display()),
        ..DevelopmentState::default()
    };
    let loaded = endpoint::load_enabled(&config.directory).await;
    let initially_enabled = loaded.as_ref().copied().unwrap_or(false);
    state.error = loaded.err();
    let mut active = None;
    if initially_enabled {
        activate(&config, &mut state, &mut active).await;
    }
    publish(&events, &state).await;
    let (incoming_tx, incoming) = async_channel::bounded::<Incoming>(8);
    let mut tasks = JoinSet::new();
    let mut pending: BTreeMap<u64, Pending> = BTreeMap::new();
    let mut sequence = 0_u64;
    let mut expiry = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => {
                let Ok(command) = command else { break };
                match command {
                    DevelopmentCommand::SetEnabled(enabled) => {
                        if !enabled {
                            // Revoke first, even if saving the preference fails.
                            tasks.abort_all();
                            pending.clear();
                            if let Some(endpoint) = active.take() {
                                endpoint.close().await;
                            }
                            state.enabled = false;
                            state.generation = None;
                            state.error = None;
                            publish(&events, &state).await;
                        } else if active.is_none() {
                            activate(&config, &mut state, &mut active).await;
                        }
                        // A failed activation must not overwrite another instance's preference.
                        if (!enabled || state.enabled)
                            && let Err(error) = endpoint::save_enabled(&config.directory, state.enabled).await
                        {
                            state.error = Some(error);
                        }
                        publish(&events, &state).await;
                    }
                    DevelopmentCommand::UiReply { generation, request_id, result } => {
                        if state.accepts(&generation)
                            && let Some(waiter) = pending.remove(&request_id)
                            && waiter.deadline > Instant::now()
                        {
                            let _ = waiter.reply.send(result);
                        }
                    }
                }
            }
            accepted = accept(active.as_ref().map(|endpoint: &endpoint::Active| &endpoint.listener)), if active.is_some() && tasks.len() < 16 => {
                match accepted {
                    Ok((stream, _)) => {
                        if let Some(endpoint) = &active {
                            let token = endpoint.descriptor.token.clone();
                            let generation = endpoint.descriptor.generation.clone();
                            let incoming = incoming_tx.clone();
                            tasks.spawn(async move {
                                let _ = tokio::time::timeout(REQUEST_TIMEOUT, endpoint::serve_connection(stream, &token, generation, incoming)).await;
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "development listener failed");
                        state.error = Some("Development listener failed; switch the mode off and on.".into());
                        publish(&events, &state).await;
                    }
                }
            }
            request = incoming.recv() => {
                if let Ok(Incoming { generation: incoming_generation, request, reply }) = request {
                    if reply.is_closed() { continue; }
                    if let Some(generation) = state.generation.clone().filter(|_| state.accepts(&incoming_generation)) {
                        if pending.len() >= 8 {
                            let _ = reply.send(Err("Development mode is busy; inspect again shortly.".into()));
                            continue;
                        }
                        sequence += 1;
                        let deadline = Instant::now() + Duration::from_secs(3);
                        pending.insert(sequence, Pending { deadline, reply });
                        let _ = events.send(Event::Development(DevelopmentEvent::UiRequest {
                            generation, request_id: sequence, request, deadline,
                        })).await;
                    } else {
                        let _ = reply.send(Err("Development mode is off.".into()));
                    }
                }
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            _ = expiry.tick() => {
                pending.retain(|_, request| request.deadline > Instant::now() && !request.reply.is_closed());
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    pending.clear();
    if let Some(endpoint) = active {
        endpoint.close().await;
    }
}

async fn accept(
    listener: Option<&TcpListener>,
) -> std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

async fn activate(
    config: &DevelopmentConfig,
    state: &mut DevelopmentState,
    active: &mut Option<endpoint::Active>,
) {
    match endpoint::Active::open(&config.directory).await {
        Ok(endpoint) => {
            state.enabled = true;
            state.generation = Some(endpoint.descriptor.generation.clone());
            state.error = None;
            *active = Some(endpoint);
        }
        Err(error) => {
            state.enabled = false;
            state.generation = None;
            state.error = Some(error);
        }
    }
}

async fn publish(events: &Sender<Event>, state: &DevelopmentState) {
    let _ = events
        .send(Event::Development(DevelopmentEvent::State(state.clone())))
        .await;
}

#[cfg(test)]
mod tests;
