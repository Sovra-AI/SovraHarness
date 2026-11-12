//! Phone control actor: persistence, owned connection tasks and bounded UI dispatch.
mod storage;
mod worker;

use async_channel::{Receiver, Sender};
use sovra_core::{
    Event,
    group::DeviceId,
    remote_control::{
        AccessError, ErrorCode, InvitationSecret, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
        PAIRING_LIFETIME_MS, PROTOCOL_VERSION, PairingTicket, RemoteAccess, RemoteCommand,
        RemoteConnection, RemoteEndpoint, RemoteEvent, RemoteOperation, RemoteRequest,
        RemoteResponse, RemoteState, SessionGuard,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::oneshot,
    task::{JoinHandle, JoinSet},
};

pub type RemoteEndpointFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn RemoteEndpoint>, String>> + Send>>;
pub type RemoteEndpointFactory = Arc<dyn Fn() -> RemoteEndpointFuture + Send + Sync>;

pub struct RemoteConfig {
    pub directory: PathBuf,
    pub transport: RemoteEndpointFactory,
    pub qr: crate::QrEncoder,
}

impl std::fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteConfig")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

pub(super) const IO_TIMEOUT: Duration = Duration::from_secs(5);
const UI_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct RemoteRuntime {
    commands: Sender<RemoteCommand>,
    events: Sender<Event>,
    task: JoinHandle<()>,
}

impl RemoteRuntime {
    pub(crate) fn start(
        config: Option<RemoteConfig>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (commands, incoming) = async_channel::bounded(32);
        let output = events.clone();
        let task =
            handle.spawn(async move { Actor::new(config, output).await.run(incoming).await });
        Self {
            commands,
            events,
            task,
        }
    }

    pub(crate) fn send(&self, command: RemoteCommand) {
        if self.commands.try_send(command).is_err() {
            let _ = self
                .events
                .try_send(Event::RemoteControl(Box::new(RemoteEvent::Error(
                    "Phone control is busy. Wait briefly and try again.".into(),
                ))));
        }
    }

    pub(crate) async fn stop(self) {
        self.commands.close();
        let _ = self.task.await;
    }
}

pub(super) enum WorkerMessage {
    Connected {
        generation: DeviceId,
        id: u64,
        connection: Arc<dyn RemoteConnection>,
    },
    Request {
        generation: DeviceId,
        id: u64,
        peer: DeviceId,
        request: RemoteRequest,
        reply: oneshot::Sender<RemoteResponse>,
    },
}

struct Controller {
    connection: u64,
    phone: DeviceId,
    session: DeviceId,
    guard: SessionGuard,
}

struct Pending {
    phone: DeviceId,
    session: DeviceId,
    deadline: Instant,
    reply: oneshot::Sender<RemoteResponse>,
}

struct Actor {
    config: Option<RemoteConfig>,
    _storage_lock: Option<std::fs::File>,
    storage_error: Option<String>,
    events: Sender<Event>,
    policy: RemoteAccess,
    state: RemoteState,
    origin: Instant,
    endpoint: Option<Arc<dyn RemoteEndpoint>>,
    tasks: JoinSet<u64>,
    connections: BTreeMap<u64, Arc<dyn RemoteConnection>>,
    live_connections: BTreeSet<u64>,
    controller: Option<Controller>,
    pending: BTreeMap<u64, Pending>,
    request_id: u64,
}

impl Actor {
    async fn new(config: Option<RemoteConfig>, events: Sender<Event>) -> Self {
        let loaded = match &config {
            Some(config) => storage::open(&config.directory)
                .await
                .map(|(lock, policy)| (Some(lock), policy)),
            None => Ok((None, RemoteAccess::default())),
        };
        let (lock, policy, error) = match loaded {
            Ok((lock, policy)) => (lock, policy, None),
            Err(error) => (None, RemoteAccess::default(), Some(error)),
        };
        let state = RemoteState {
            phones: policy.phones().to_vec(),
            error: error.clone(),
            ..RemoteState::default()
        };
        Self {
            config,
            _storage_lock: lock,
            storage_error: error,
            events,
            policy,
            state,
            origin: Instant::now(),
            endpoint: None,
            tasks: JoinSet::new(),
            connections: BTreeMap::new(),
            live_connections: BTreeSet::new(),
            controller: None,
            pending: BTreeMap::new(),
            request_id: 0,
        }
    }

    async fn run(mut self, commands: Receiver<RemoteCommand>) {
        if self.config.is_some() {
            self.publish().await;
        }
        let (messages, inbox) = async_channel::bounded(32);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut admissions = 0;
        let mut requests = 0;
        let mut connection_id = 0_u64;
        loop {
            tokio::select! {
                biased;
                command = commands.recv() => {
                    let Ok(command) = command else { break };
                    self.command(command).await;
                }
                _ = tick.tick() => {
                    admissions = 0;
                    requests = 0;
                    self.expire().await;
                }
                finished = self.tasks.join_next(), if !self.tasks.is_empty() => {
                    if let Some(Ok(id)) = finished {
                        self.live_connections.remove(&id);
                        self.connections.remove(&id);
                        if self.controller.as_ref().is_some_and(|owner| owner.connection == id) {
                            self.clear_controller();
                            self.publish().await;
                        }
                    } else if finished.is_some() {
                        self.disable().await;
                        self.state.error = Some("Phone connection worker stopped. Start remote control again.".into());
                        self.publish().await;
                    }
                }
                message = inbox.recv() => {
                    if let Ok(message) = message {
                        match message {
                            WorkerMessage::Connected { generation, id, connection } => {
                                self.record_connection(generation, id, connection);
                            }
                            WorkerMessage::Request { generation, id, peer, request, reply } => {
                                requests += 1;
                                if self.state.generation != Some(generation) || !self.live_connections.contains(&id) {
                                    let _ = reply.send(failure(ErrorCode::Disabled, "Remote control stopped."));
                                } else if requests > 32 {
                                    let _ = reply.send(failure(ErrorCode::Busy, "Too many requests. Wait briefly."));
                                } else {
                                    self.incoming(id, peer, request, reply).await;
                                }
                            }
                        }
                    }
                }
                accepted = accept(self.endpoint.as_ref()), if self.endpoint.is_some() && self.tasks.len() < 16 && admissions < 16 => {
                    admissions += 1;
                    if let Ok(handshake) = accepted {
                        let Some(id) = connection_id.checked_add(1) else { break };
                        connection_id = id;
                        self.live_connections.insert(id);
                        let generation = self.state.generation.expect("active endpoint has an activation");
                        let messages = messages.clone();
                        self.tasks.spawn(worker::serve(id, generation, handshake, messages));
                    } else {
                        self.disable().await;
                        self.state.error = Some("Phone connection stopped. Start remote control again.".into());
                        self.publish().await;
                    }
                }
            }
        }
        self.disable().await;
    }

    fn record_connection(
        &mut self,
        generation: DeviceId,
        id: u64,
        connection: Arc<dyn RemoteConnection>,
    ) {
        if self.state.generation == Some(generation) && self.live_connections.contains(&id) {
            self.connections.insert(id, connection);
        } else {
            connection.close();
        }
    }

    async fn command(&mut self, command: RemoteCommand) {
        match command {
            RemoteCommand::Start => {
                if self.endpoint.is_none()
                    && let Err(error) = self.enable().await
                {
                    self.state.error = Some(error);
                }
                self.publish().await;
            }
            RemoteCommand::Stop => {
                self.disable().await;
                self.state.error = None;
                self.publish().await;
            }
            RemoteCommand::PairAnother => {
                if let Err(error) = self.invite() {
                    self.state.error = Some(error);
                }
                self.publish().await;
            }
            RemoteCommand::Forget { device } => {
                let mut candidate = self.policy.clone();
                candidate.forget(device);
                if let Err(error) = self.save(&candidate).await {
                    self.disable().await;
                    self.state.error = Some(format!(
                        "Phone was not forgotten. Remote control stopped. Retry Forget before restarting. {error}"
                    ));
                } else {
                    self.policy = candidate;
                    for connection in self
                        .connections
                        .values()
                        .filter(|connection| connection.peer() == device)
                    {
                        connection.close();
                    }
                    if self
                        .controller
                        .as_ref()
                        .is_some_and(|owner| owner.phone == device)
                    {
                        self.clear_controller();
                    }
                    self.state.error = None;
                }
                self.publish().await;
            }
            RemoteCommand::UiReply {
                generation,
                request_id,
                result,
            } => {
                if let Some(pending) = self.pending.remove(&request_id) {
                    if pending.deadline > Instant::now()
                        && self
                            .state
                            .accepts(generation, pending.phone, pending.session)
                        && self
                            .policy
                            .authorize(PROTOCOL_VERSION, pending.phone, &generation.0)
                            .is_ok()
                    {
                        let response = match result {
                            Ok(value) => RemoteResponse::Result { value },
                            Err(error) => failure(error.code, &error.message),
                        };
                        let _ = pending.reply.send(response);
                    } else {
                        let _ = pending.reply.send(failure(
                            ErrorCode::OutcomeUnknown,
                            "Request expired or access changed. Inspect before retrying.",
                        ));
                    }
                }
            }
        }
    }

    async fn enable(&mut self) -> Result<(), String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        let config = self
            .config
            .as_ref()
            .ok_or("Phone control is unavailable in this session.")?;
        let generation = nonce()?;
        let endpoint = tokio::time::timeout(IO_TIMEOUT, (config.transport)())
            .await
            .map_err(|_| "Phone connection setup timed out.")??;
        self.policy.enable(generation.0);
        self.state.generation = Some(generation);
        self.state.error = None;
        self.endpoint = Some(endpoint);
        if self.policy.phones().is_empty()
            && let Err(error) = self.invite()
        {
            self.disable().await;
            return Err(error);
        }
        Ok(())
    }

    fn invite(&mut self) -> Result<(), String> {
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or("Start remote control first.")?;
        let secret = InvitationSecret(nonce()?.0);
        let ticket = PairingTicket {
            version: PROTOCOL_VERSION,
            address: endpoint.address(),
            secret,
        };
        let link = ticket.encode().map_err(|error| error.to_string())?;
        let config = self
            .config
            .as_ref()
            .ok_or("Phone control is unavailable.")?;
        let qr = (config.qr)(&link);
        self.policy
            .invite(&ticket.secret, self.now_ms())
            .map_err(|error| error.to_string())?;
        self.state.qr = qr;
        self.state.invitation = Some(ticket);
        self.state.invitation_expires_at =
            Some(Instant::now() + Duration::from_millis(PAIRING_LIFETIME_MS));
        self.state.error = None;
        Ok(())
    }

    async fn disable(&mut self) {
        self.policy.disable();
        self.state.generation = None;
        self.clear_invitation();
        self.clear_controller();
        for connection in self.connections.values() {
            connection.close();
        }
        self.connections.clear();
        self.live_connections.clear();
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        if let Some(endpoint) = self.endpoint.take() {
            let _ = tokio::time::timeout(IO_TIMEOUT, endpoint.close()).await;
        }
    }

    fn clear_invitation(&mut self) {
        self.state.invitation = None;
        self.state.invitation_expires_at = None;
        self.state.qr.clear();
    }

    fn clear_controller(&mut self) {
        self.controller = None;
        self.state.connected_phone = None;
        self.state.session = None;
        self.pending.clear();
    }

    async fn publish(&mut self) {
        self.state.phones = self.policy.phones().to_vec();
        let _ = self
            .events
            .send(Event::RemoteControl(Box::new(RemoteEvent::State(
                self.state.clone(),
            ))))
            .await;
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    async fn save(&self, candidate: &RemoteAccess) -> Result<(), String> {
        if let Some(error) = &self.storage_error {
            return Err(error.clone());
        }
        let config = self
            .config
            .as_ref()
            .ok_or("Phone control is unavailable.")?;
        crate::private_storage::write_private(&config.directory, "phones.json", &candidate.phones())
            .await
    }

    async fn expire(&mut self) {
        let now = Instant::now();
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now || pending.reply.is_closed())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(pending) = self.pending.remove(&id) {
                let _ = pending.reply.send(failure(
                    ErrorCode::OutcomeUnknown,
                    "The request expired. Inspect before retrying.",
                ));
            }
        }
        if self
            .state
            .invitation_expires_at
            .is_some_and(|expiry| expiry <= now)
        {
            self.clear_invitation();
            self.publish().await;
        }
    }

    async fn incoming(
        &mut self,
        id: u64,
        peer: DeviceId,
        request: RemoteRequest,
        reply: oneshot::Sender<RemoteResponse>,
    ) {
        if reply.is_closed() {
            return;
        }
        if request.version != PROTOCOL_VERSION {
            let _ = reply.send(failure(
                ErrorCode::UnsupportedVersion,
                "Update Sovra and the phone app to compatible versions.",
            ));
            return;
        }
        let response = match &request.operation {
            RemoteOperation::Pair { secret, name } => {
                let mut candidate = self.policy.clone();
                match candidate.pair(request.version, peer, name, secret, self.now_ms()) {
                    Err(error) => access_failure(error),
                    Ok(()) => {
                        if let Err(error) = self.save(&candidate).await {
                            failure(ErrorCode::StorageUnavailable, &error)
                        } else {
                            self.policy = candidate;
                            self.clear_invitation();
                            self.publish().await;
                            RemoteResponse::Paired {}
                        }
                    }
                }
            }
            RemoteOperation::Hello {} => self.hello(id, peer).await,
            RemoteOperation::Control { .. } => {
                self.control(id, peer, &request, reply).await;
                return;
            }
        };
        let _ = reply.send(response);
    }

    async fn hello(&mut self, id: u64, peer: DeviceId) -> RemoteResponse {
        let Some(generation) = self.state.generation else {
            return access_failure(AccessError::Disabled);
        };
        if let Err(error) = self.policy.authorize(PROTOCOL_VERSION, peer, &generation.0) {
            return access_failure(error);
        }
        if let Some(owner) = &self.controller {
            if owner.phone != peer {
                return failure(ErrorCode::Busy, "Another phone is controlling Sovra.");
            }
            if owner.connection != id
                && let Some(previous) = self.connections.get(&owner.connection)
            {
                previous.close();
            }
        }
        let session = match nonce() {
            Ok(session) => session,
            Err(error) => return failure(ErrorCode::StorageUnavailable, &error),
        };
        self.clear_controller();
        self.controller = Some(Controller {
            connection: id,
            phone: peer,
            session,
            guard: SessionGuard::new(generation, session),
        });
        self.state.connected_phone = Some(peer);
        self.state.session = Some(session);
        self.publish().await;
        RemoteResponse::Ready {
            version: PROTOCOL_VERSION,
            generation,
            session,
            max_request_bytes: MAX_REQUEST_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }

    async fn control(
        &mut self,
        id: u64,
        peer: DeviceId,
        envelope: &RemoteRequest,
        reply: oneshot::Sender<RemoteResponse>,
    ) {
        let RemoteOperation::Control {
            generation,
            revision,
            ..
        } = &envelope.operation
        else {
            return;
        };
        if let Err(error) = self.policy.authorize(envelope.version, peer, &generation.0) {
            let _ = reply.send(access_failure(error));
            return;
        }
        let Some(owner) = self
            .controller
            .as_mut()
            .filter(|owner| owner.connection == id && owner.phone == peer)
        else {
            let _ = reply.send(failure(
                ErrorCode::StaleSession,
                "Connect with Hello before controlling Sovra.",
            ));
            return;
        };
        let request = match owner.guard.admit(envelope) {
            Ok(request) => request.clone(),
            Err(code) => {
                let _ = reply.send(failure(
                    code,
                    "Refresh the connection and inspect before retrying.",
                ));
                return;
            }
        };
        if self.state.invitation.is_some() {
            let _ = reply.send(failure(
                ErrorCode::Busy,
                "Phone control pauses while a new phone is being paired.",
            ));
            return;
        }
        if self.pending.len() >= 8 {
            let _ = reply.send(failure(
                ErrorCode::Busy,
                "Sovra is busy. Inspect again shortly.",
            ));
            return;
        }
        let Some(request_id) = self.request_id.checked_add(1) else {
            let _ = reply.send(failure(ErrorCode::StaleSession, "Restart remote control."));
            return;
        };
        self.request_id = request_id;
        let deadline = Instant::now() + UI_TIMEOUT;
        self.pending.insert(
            request_id,
            Pending {
                phone: peer,
                session: owner.session,
                deadline,
                reply,
            },
        );
        let _ = self
            .events
            .send(Event::RemoteControl(Box::new(RemoteEvent::UiRequest {
                generation: *generation,
                phone: peer,
                session: owner.session,
                request_id,
                revision: revision.clone(),
                request,
                deadline,
            })))
            .await;
    }
}

async fn accept(
    endpoint: Option<&Arc<dyn RemoteEndpoint>>,
) -> Result<Box<dyn sovra_core::remote_control::RemoteHandshake>, sovra_core::group::TransportError>
{
    match endpoint {
        Some(endpoint) => endpoint.accept().await,
        None => std::future::pending().await,
    }
}

pub(super) fn failure(code: ErrorCode, message: &str) -> RemoteResponse {
    RemoteResponse::Error {
        code,
        message: message.into(),
    }
}

fn access_failure(error: AccessError) -> RemoteResponse {
    let code = match error {
        AccessError::Disabled => ErrorCode::Disabled,
        AccessError::UnsupportedVersion => ErrorCode::UnsupportedVersion,
        AccessError::NotPaired => ErrorCode::NotPaired,
        AccessError::StaleGeneration => ErrorCode::StaleSession,
        AccessError::InvalidInvitation => ErrorCode::InvalidInvitation,
        AccessError::PhoneLimit | AccessError::InvalidName | AccessError::DuplicatePhone => {
            ErrorCode::InvalidRequest
        }
    };
    failure(code, &error.to_string())
}

fn nonce() -> Result<DeviceId, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "Secure randomness is unavailable.".to_owned())?;
    Ok(DeviceId(bytes))
}

#[cfg(test)]
mod tests;
