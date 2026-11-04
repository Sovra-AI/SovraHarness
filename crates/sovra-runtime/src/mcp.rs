//! Connected services: web addresses and programs on this computer.
//!
//! The screen only sends [`sovra_core::mcp::McpCommand`]. This task owns HTTP,
//! the loopback sign-in listeners, the saved list and the credential store.
//! Tokens, header values and program settings are never written to events or
//! logs. Slow work never holds up the actor: tool calls, sign-in waits and every
//! program run in their own tasks, and reconnects are queued one per message so
//! other commands interleave. Design: `docs/connected-services.md`.

mod programs;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use sovra_core::Event;
use sovra_core::auth::{callback_page, constant_time_str_eq};
use sovra_core::mcp::{
    ACCEPT_HEADER, AuthorizationServer, ConnectFailure, DraftSetup, ImportProblem,
    InitializeOutcome, MAX_SERVICES, METHOD_HEADER, McpCommand, McpEvent, McpPhase, McpService,
    McpState, McpTransport, NAME_HEADER, PROTOCOL_HEADER, PROTOCOL_LEGACY, PROTOCOL_MODERN,
    RegistrationChoice, RegistrationOffer, RemoteCall, RemoteOutput, RemoteToolCaller, SecretKind,
    SecretRecord, SignInCallback, TOOL_FAILED_TEXT, ToolOffer, accept_secret, approval_fingerprint,
    authorization_issuer_ok, authorization_server_metadata_url, check_service_address,
    choose_registration, dynamic_registration_document, env_name_allowed, header_allowed,
    header_field_value, initialize_request, initialized_notification, interpret_initialize,
    json_from_body, mirrored_name, name_for_program, parse_authorization_server,
    parse_protected_resource, parse_server_settings, parse_sign_in_callback, parse_tool_list,
    parse_tool_text, program_display, program_id_for, protected_resource_well_known_urls,
    request_origin, resource_metadata_parameter, rpc_result, s256_challenge, secret_storage_id,
    service_id_for, token_may_be_sent, tools_call_request, tools_list_request,
};
use sovra_core::models::ModelSecret;
use sovra_providers::{ModelRepository, SignInProof};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

const CATALOGUE_ID: &str = "mcp-catalogue";
const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
const LOOPBACK_LIMIT: usize = 8 * 1024;
const CATALOGUE_VERSION: u32 = 1;

type CallReply = tokio::sync::oneshot::Sender<Result<RemoteOutput, String>>;

enum Input {
    Command(McpCommand),
    Invoke {
        request: RemoteCall,
        reply: CallReply,
    },
    /// News from a program task.
    Program(programs::ProgramUpdate),
    /// A sign-in page came back (or timed out) for one service.
    SignedIn {
        id: String,
        pending: Box<PendingSignIn>,
        outcome: Result<SignInCallback, ConnectFailure>,
    },
}

/// Everything needed to finish a sign-in once the browser comes back.
struct PendingSignIn {
    server: AuthorizationServer,
    material: ClientMaterial,
    redirect: String,
    resource: String,
    proof: SignInProof,
}

/// Connected tools a chat turn may call. The actor replaces the list; readers clone it.
#[derive(Debug, Default)]
struct ToolBoard {
    offers: RwLock<Vec<ToolOffer>>,
}

impl ToolBoard {
    fn snapshot(&self) -> Vec<ToolOffer> {
        self.offers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn replace(&self, offers: Vec<ToolOffer>) {
        *self
            .offers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = offers;
    }
}

/// Forwards a chat-turn tool call to the actor. Holds no secrets.
#[derive(Clone)]
pub(crate) struct ServiceCaller {
    inbox: Sender<Input>,
}

impl std::fmt::Debug for ServiceCaller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServiceCaller")
    }
}

#[async_trait::async_trait]
impl RemoteToolCaller for ServiceCaller {
    async fn call(&self, request: RemoteCall) -> Result<RemoteOutput, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.inbox
            .send(Input::Invoke { request, reply: tx })
            .await
            .map_err(|_| TOOL_FAILED_TEXT.to_owned())?;
        rx.await.map_err(|_| TOOL_FAILED_TEXT.to_owned())?
    }
}

pub(crate) struct McpRuntime {
    inbox: Sender<Input>,
    task: tokio::task::JoinHandle<()>,
    board: Arc<ToolBoard>,
    caller: Arc<ServiceCaller>,
}

impl McpRuntime {
    pub(crate) fn start(
        repository: Option<ModelRepository>,
        catalogue_path: Option<PathBuf>,
        environment: Option<crate::terminal::LoginEnvironment>,
        events: Sender<Event>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let (inbox, rx) = async_channel::unbounded();
        let board = Arc::new(ToolBoard::default());
        let caller = Arc::new(ServiceCaller {
            inbox: inbox.clone(),
        });
        let actor = Actor {
            handle: handle.clone(),
            inbox: inbox.downgrade(),
            events,
            http: http_client(),
            vault: Vault::new(repository),
            catalogue_path,
            environment,
            services: Vec::new(),
            board: Arc::clone(&board),
            next_rpc: 1,
            listeners: HashMap::new(),
            programs: HashMap::new(),
        };
        let task = handle.spawn(actor.run(rx));
        Self {
            inbox,
            task,
            board,
            caller,
        }
    }

    pub(crate) fn handle(&self, command: McpCommand) {
        let _ = self.inbox.try_send(Input::Command(command));
    }

    pub(crate) fn offers(&self) -> Vec<ToolOffer> {
        self.board.snapshot()
    }

    pub(crate) fn caller(&self) -> Arc<ServiceCaller> {
        Arc::clone(&self.caller)
    }

    pub(crate) async fn stop(&mut self) {
        self.inbox.close();
        let _ = (&mut self.task).await;
    }
}

impl Drop for McpRuntime {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("the HTTP client uses the rustls defaults and does not follow redirects")
}

struct Actor {
    handle: tokio::runtime::Handle,
    /// Weak, so the actor never keeps its own inbox open after the runtime stops.
    inbox: async_channel::WeakSender<Input>,
    events: Sender<Event>,
    http: reqwest::Client,
    vault: Vault,
    /// The saved list; `None` keeps it in the credential store (tests).
    catalogue_path: Option<PathBuf>,
    environment: Option<crate::terminal::LoginEnvironment>,
    services: Vec<Live>,
    board: Arc<ToolBoard>,
    next_rpc: u64,
    /// One sign-in waiter per service, so several sign-ins can be open at once.
    listeners: HashMap<String, tokio::task::JoinHandle<()>>,
    /// Running programs by service id. Dropping one stops it.
    programs: HashMap<String, programs::ProgramHandle>,
}

struct Live {
    service: McpService,
    protocol: String,
    session: Option<String>,
    issuer: String,
    auth: Option<AuthorizationServer>,
    named_by_person: bool,
    uses_static: bool,
    /// Fingerprint of the program the person approved (see `approval_fingerprint`).
    approved: Option<String>,
    /// Extra headers for a web address, from the keychain. Never in events.
    headers: Vec<(String, String)>,
}

impl Live {
    fn new(service: McpService) -> Self {
        Self {
            service,
            protocol: PROTOCOL_MODERN.to_owned(),
            session: None,
            issuer: String::new(),
            auth: None,
            named_by_person: true,
            uses_static: false,
            approved: None,
            headers: Vec::new(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CatalogueEntry {
    id: String,
    name: String,
    address: String,
    issuer: String,
    #[serde(default)]
    transport: McpTransport,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    disabled_tools: Vec<String>,
    #[serde(default)]
    header_names: Vec<String>,
    #[serde(default)]
    approved: Option<String>,
}

const fn enabled_by_default() -> bool {
    true
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CatalogueFile {
    version: u32,
    services: Vec<CatalogueEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedClient {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

struct ClientMaterial {
    client_id: String,
    client_secret: Option<String>,
}

struct Reply {
    status: u16,
    session: Option<String>,
    www_authenticate: String,
    location: Option<String>,
    body: String,
}

enum Vault {
    Memory(HashMap<String, String>),
    Repository(ModelRepository),
}

impl Vault {
    fn new(repository: Option<ModelRepository>) -> Self {
        repository.map_or_else(|| Self::Memory(HashMap::new()), Self::Repository)
    }

    async fn get(&self, id: &str) -> Option<String> {
        match self {
            Self::Memory(map) => map.get(id).filter(|value| !value.is_empty()).cloned(),
            Self::Repository(repository) => {
                let repository = repository.clone();
                let id = id.to_owned();
                let secret = tokio::task::spawn_blocking(move || repository.named_secret(&id))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .unwrap_or_default();
                (!secret.0.is_empty()).then_some(secret.0)
            }
        }
    }

    async fn put(&mut self, id: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        match self {
            Self::Memory(map) => {
                map.insert(id.to_owned(), value.to_owned());
            }
            Self::Repository(repository) => {
                let repository = repository.clone();
                let id = id.to_owned();
                let secret = ModelSecret(value.to_owned());
                let _ =
                    tokio::task::spawn_blocking(move || repository.set_credential(&id, &secret))
                        .await;
            }
        }
    }

    async fn delete(&mut self, id: &str) {
        match self {
            Self::Memory(map) => {
                map.remove(id);
            }
            Self::Repository(repository) => {
                let repository = repository.clone();
                let id = id.to_owned();
                let _ =
                    tokio::task::spawn_blocking(move || repository.delete_credential(&id)).await;
            }
        }
    }
}

impl Actor {
    async fn run(mut self, rx: Receiver<Input>) {
        while let Ok(input) = rx.recv().await {
            match input {
                Input::Command(command) => self.command(command).await,
                Input::Invoke { request, reply } => self.perform_call(request, Some(reply)).await,
                Input::Program(update) => self.program_update(update).await,
                Input::SignedIn {
                    id,
                    pending,
                    outcome,
                } => self.complete_sign_in(id, *pending, outcome).await,
            }
        }
        for (_, listener) in self.listeners.drain() {
            listener.abort();
        }
        // Dropping the handles stops every program and its children.
        self.programs.clear();
    }

    /// Queues a command behind whatever is already waiting.
    fn queue(&self, command: McpCommand) {
        if let Some(inbox) = self.inbox.upgrade() {
            let _ = inbox.try_send(Input::Command(command));
        }
    }

    async fn command(&mut self, command: McpCommand) {
        match command {
            McpCommand::Load => self.load().await,
            McpCommand::Connect {
                name,
                address,
                secret,
            } => self.connect(name, address, secret).await,
            McpCommand::SubmitClientDetails {
                id,
                client_id,
                client_secret,
            } => self.submit_client(id, client_id, client_secret).await,
            McpCommand::SignIn { id } => self.sign_in(id).await,
            McpCommand::Remove { id } => self.remove(id).await,
            McpCommand::AddProgram {
                name,
                command,
                args,
                env,
                working_dir,
            } => {
                if self
                    .add_program(&name, command, args, env, working_dir, true)
                    .await
                {
                    self.emit(McpEvent::Notice(
                        "Added. Check the program below and choose Run it when you're happy."
                            .to_owned(),
                    ))
                    .await;
                }
            }
            McpCommand::Import { text } => self.import(&text).await,
            McpCommand::Approve { id } => self.approve(&id).await,
            McpCommand::SetEnabled { id, enabled } => self.set_enabled(&id, enabled).await,
            McpCommand::SetToolEnabled { id, tool, enabled } => {
                self.set_tool_enabled(&id, tool, enabled).await;
            }
            McpCommand::Reconnect { id } => self.reconnect(&id).await,
            McpCommand::Rename { id, name } => self.rename(&id, &name).await,
            McpCommand::CallTool {
                service_id,
                tool,
                arguments,
                session_id,
            } => {
                let request = RemoteCall {
                    service_id,
                    tool,
                    arguments,
                    session_id: session_id.unwrap_or_default(),
                };
                self.perform_call(request, None).await;
            }
        }
    }

    async fn load(&mut self) {
        let entries = self.read_catalogue().await;
        self.services = Vec::with_capacity(entries.len());
        for entry in entries {
            let mut live = Live::new(McpService {
                id: entry.id,
                name: entry.name,
                address: entry.address,
                phase: McpPhase::Saved,
                message: McpPhase::Saved.label().to_owned(),
                transport: entry.transport,
                enabled: entry.enabled,
                disabled_tools: entry.disabled_tools,
                header_names: entry.header_names,
                ..McpService::default()
            });
            live.issuer = entry.issuer;
            live.approved = entry.approved;
            if !live.service.header_names.is_empty() {
                live.headers = self.read_settings(&live.service.id).await;
            }
            self.services.push(live);
        }
        self.sync_board();
        self.emit(McpEvent::Loaded(self.view_state())).await;
        // One queued reconnect per service: a slow one never holds up the others,
        // and tool calls and screen commands interleave between them.
        for id in self.enabled_ids() {
            self.queue(McpCommand::Reconnect { id });
        }
    }

    fn enabled_ids(&self) -> Vec<String> {
        self.services
            .iter()
            .filter(|live| live.service.enabled)
            .map(|live| live.service.id.clone())
            .collect()
    }

    async fn read_catalogue(&mut self) -> Vec<CatalogueEntry> {
        if let Some(path) = &self.catalogue_path {
            match tokio::fs::read(path).await {
                Ok(bytes) => {
                    return match serde_json::from_slice::<CatalogueFile>(&bytes) {
                        Ok(file) if file.version == CATALOGUE_VERSION => file.services,
                        Ok(_) | Err(_) => {
                            tracing::warn!("unreadable connected services list; starting empty");
                            Vec::new()
                        }
                    };
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(%error, "could not read connected services list");
                    return Vec::new();
                }
            }
        }
        // Older builds kept the list in the credential store, where Windows limits
        // each entry to about 2.5 KB. Move it to the file once.
        let raw = self.vault.get(CATALOGUE_ID).await.unwrap_or_default();
        let entries: Vec<CatalogueEntry> = serde_json::from_str(&raw).unwrap_or_default();
        if self.catalogue_path.is_some()
            && !entries.is_empty()
            && self.write_catalogue_file(&entries).await
        {
            self.vault.delete(CATALOGUE_ID).await;
            tracing::info!(
                services = entries.len(),
                "moved connected services list to its file"
            );
        }
        entries
    }

    /// Private settings (program environment or web headers) for one service.
    async fn read_settings(&self, id: &str) -> Vec<(String, String)> {
        self.read_secret("", id, SecretKind::Settings)
            .await
            .and_then(|raw| serde_json::from_str::<Vec<(String, String)>>(&raw).ok())
            .unwrap_or_default()
    }

    async fn store_settings(&mut self, id: &str, pairs: &[(String, String)]) {
        let key = secret_storage_id("", id, SecretKind::Settings);
        if pairs.is_empty() {
            self.vault.delete(&key).await;
            return;
        }
        if let Ok(encoded) = serde_json::to_string(pairs) {
            self.store_secret("", id, SecretKind::Settings, &encoded)
                .await;
        }
    }

    fn has_room(&self, id: &str) -> bool {
        self.services.len() < MAX_SERVICES || self.live(id).is_some()
    }

    /// Adds a program on this computer. It waits for the person's OK before it
    /// first runs. Returns whether it was added.
    async fn add_program(
        &mut self,
        name: &str,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        working_dir: Option<String>,
        enabled: bool,
    ) -> bool {
        let command = command.trim().to_owned();
        if command.is_empty() {
            self.emit(McpEvent::Notice(ImportProblem::Empty.text().to_owned()))
                .await;
            return false;
        }
        let env: Vec<(String, String)> = env
            .into_iter()
            .map(|(name, value)| (name.trim().to_owned(), value))
            .filter(|(name, _)| env_name_allowed(name))
            .collect();
        let id = program_id_for(&command, &args);
        if !self.has_room(&id) {
            self.emit(McpEvent::Notice(FULL_TEXT.to_owned())).await;
            return false;
        }
        self.programs.remove(&id);
        self.store_settings(&id, &env).await;
        let display = if name.trim().is_empty() {
            name_for_program(&command, &args)
        } else {
            name.trim().to_owned()
        };
        let address = program_display(&command, &args);
        let transport = McpTransport::Program {
            command,
            args,
            env_names: env.iter().map(|(name, _)| name.clone()).collect(),
            working_dir: working_dir
                .map(|dir| dir.trim().to_owned())
                .filter(|dir| !dir.is_empty()),
        };
        self.services.retain(|live| live.service.id != id);
        self.services.push(Live::new(McpService {
            id: id.clone(),
            name: display,
            address,
            phase: if enabled {
                McpPhase::NeedsApproval
            } else {
                McpPhase::Saved
            },
            message: McpPhase::NeedsApproval.label().to_owned(),
            transport,
            enabled,
            ..McpService::default()
        }));
        tracing::info!(service_id = %id, "program added, waiting for approval");
        self.publish(&id).await;
        true
    }

    /// Adds a web address without connecting yet (pasted settings); a queued
    /// reconnect connects it.
    async fn add_web(
        &mut self,
        name: &str,
        address: &str,
        headers: Vec<(String, String)>,
        enabled: bool,
    ) -> Result<String, String> {
        let checked =
            check_service_address(address).map_err(|problem| problem.text().to_owned())?;
        let id = service_id_for(&checked.url);
        if !self.has_room(&id) {
            return Err(FULL_TEXT.to_owned());
        }
        let headers: Vec<(String, String)> = headers
            .into_iter()
            .filter(|(name, value)| header_allowed(name, value))
            .collect();
        self.store_settings(&id, &headers).await;
        let named_by_person = !name.trim().is_empty();
        let mut live = Live::new(McpService {
            id: id.clone(),
            name: if named_by_person {
                name.trim().to_owned()
            } else {
                checked.host.clone()
            },
            address: checked.url.clone(),
            phase: McpPhase::Saved,
            message: McpPhase::Saved.label().to_owned(),
            enabled,
            header_names: headers.iter().map(|(name, _)| name.clone()).collect(),
            ..McpService::default()
        });
        live.named_by_person = named_by_person;
        live.headers = headers;
        self.services.retain(|existing| existing.service.id != id);
        self.services.push(live);
        self.publish(&id).await;
        Ok(id)
    }

    async fn import(&mut self, text: &str) {
        let drafts = match parse_server_settings(text) {
            Ok(drafts) => drafts,
            Err(problem) => {
                self.emit(McpEvent::Notice(problem.text().to_owned())).await;
                return;
            }
        };
        let mut added = 0_usize;
        let mut programs = 0_usize;
        let mut problems = Vec::new();
        for draft in drafts {
            match draft.setup {
                DraftSetup::Web { address, headers } => {
                    match self
                        .add_web(&draft.name, &address, headers, draft.enabled)
                        .await
                    {
                        Ok(id) => {
                            added += 1;
                            if draft.enabled {
                                self.queue(McpCommand::Reconnect { id });
                            }
                        }
                        Err(problem) => problems
                            .push(format!("{}: {problem}", label_for(&draft.name, &address))),
                    }
                }
                DraftSetup::Program {
                    command,
                    args,
                    env,
                    working_dir,
                } => {
                    if self
                        .add_program(&draft.name, command, args, env, working_dir, draft.enabled)
                        .await
                    {
                        added += 1;
                        programs += 1;
                    }
                }
            }
        }
        let mut notice = match added {
            0 => "Nothing was added.".to_owned(),
            1 => "Added 1 service.".to_owned(),
            count => format!("Added {count} services."),
        };
        if programs > 0 {
            notice.push_str(" Programs wait for your OK before they run.");
        }
        for problem in problems.iter().take(3) {
            notice.push('\n');
            notice.push_str(problem);
        }
        tracing::info!(
            added,
            programs,
            problems = problems.len(),
            "imported services"
        );
        self.emit(McpEvent::Notice(notice)).await;
    }

    async fn approve(&mut self, id: &str) {
        let Some(live) = self.live_mut(id) else {
            return;
        };
        let Some(fingerprint) = approval_fingerprint(&live.service.transport) else {
            return;
        };
        live.approved = Some(fingerprint);
        live.service.enabled = true;
        tracing::info!(service_id = %id, "program approved");
        self.start_program(id).await;
    }

    async fn start_program(&mut self, id: &str) {
        let Some(live) = self.live(id) else {
            return;
        };
        let McpTransport::Program {
            command,
            args,
            working_dir,
            ..
        } = live.service.transport.clone()
        else {
            return;
        };
        if live.approved != approval_fingerprint(&live.service.transport) {
            self.mark(id, McpPhase::NeedsApproval, McpPhase::NeedsApproval.label())
                .await;
            return;
        }
        self.programs.remove(id);
        self.mark(id, McpPhase::Connecting, "Starting…").await;
        let Some(updates) = self.inbox.upgrade() else {
            return;
        };
        let spec = programs::ProgramSpec {
            id: id.to_owned(),
            program: command,
            args,
            private_env: self.read_settings(id).await,
            working_dir,
            environment: self.environment.clone(),
        };
        let handle = programs::start(spec, updates, &self.handle);
        self.programs.insert(id.to_owned(), handle);
    }

    async fn program_update(&mut self, update: programs::ProgramUpdate) {
        match update {
            programs::ProgramUpdate::Connected {
                id,
                server_name,
                tools,
            } => {
                if self.programs.contains_key(&id) {
                    self.store_catalogue(&id, PROTOCOL_MODERN, None, server_name, tools)
                        .await;
                }
            }
            programs::ProgramUpdate::Stopped { id, detail } => {
                self.programs.remove(&id);
                if self.live(&id).is_some_and(|live| live.service.enabled) {
                    self.mark(&id, McpPhase::Failed, &detail).await;
                }
            }
        }
    }

    async fn set_enabled(&mut self, id: &str, enabled: bool) {
        let Some(live) = self.live_mut(id) else {
            return;
        };
        if live.service.enabled == enabled {
            return;
        }
        live.service.enabled = enabled;
        if enabled {
            tracing::info!(service_id = %id, "service resumed");
            self.reconnect(id).await;
            return;
        }
        live.service.phase = McpPhase::Saved;
        sovra_core::mcp::PAUSED_LABEL.clone_into(&mut live.service.message);
        live.service.tools.clear();
        live.session = None;
        self.programs.remove(id);
        if let Some(listener) = self.listeners.remove(id) {
            listener.abort();
        }
        tracing::info!(service_id = %id, "service paused");
        self.publish(id).await;
    }

    async fn set_tool_enabled(&mut self, id: &str, tool: String, enabled: bool) {
        let Some(live) = self.live_mut(id) else {
            return;
        };
        live.service.disabled_tools.retain(|name| *name != tool);
        if !enabled {
            live.service.disabled_tools.push(tool);
            live.service.disabled_tools.sort();
        }
        self.publish(id).await;
    }

    async fn rename(&mut self, id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let Some(live) = self.live_mut(id) else {
            return;
        };
        name.clone_into(&mut live.service.name);
        live.named_by_person = true;
        self.publish(id).await;
    }

    /// Connects again with what is saved: restarts a program, or retries a web
    /// address with its saved sign-in or secret.
    async fn reconnect(&mut self, id: &str) {
        let Some(live) = self.live(id) else {
            return;
        };
        if !live.service.enabled {
            return;
        }
        if live.service.transport.is_program() {
            self.start_program(id).await;
            return;
        }
        self.mark(id, McpPhase::Connecting, McpPhase::Connecting.label())
            .await;
        let bearer = self.saved_bearer(id).await;
        let authorized = bearer.is_some();
        self.establish(id, bearer, authorized, false).await;
    }

    async fn saved_bearer(&mut self, id: &str) -> Option<String> {
        let live = self.live(id)?;
        let (issuer, address) = (live.issuer.clone(), live.service.address.clone());
        if !issuer.is_empty() {
            return self
                .read_secret(&issuer, &address, SecretKind::Access)
                .await;
        }
        let secret = self.read_secret("", &address, SecretKind::Static).await;
        if secret.is_some()
            && let Some(live) = self.live_mut(id)
        {
            live.uses_static = true;
        }
        secret
    }

    async fn connect(&mut self, name: String, address: String, secret: Option<String>) {
        let checked = match check_service_address(&address) {
            Ok(checked) => checked,
            Err(problem) => {
                self.emit(McpEvent::Notice(problem.text().to_owned())).await;
                return;
            }
        };
        let id = service_id_for(&checked.url);
        if !self.has_room(&id) {
            self.emit(McpEvent::Notice(FULL_TEXT.to_owned())).await;
            return;
        }
        self.programs.remove(&id);
        if let Some(secret) = secret
            .as_deref()
            .map(str::trim)
            .filter(|secret| !secret.is_empty())
        {
            self.store_secret("", &checked.url, SecretKind::Static, secret)
                .await;
        }
        let named_by_person = !name.trim().is_empty();
        let display = if named_by_person {
            name.trim().to_owned()
        } else {
            checked.host.clone()
        };
        // Reconnecting through the form keeps the person's tool switches.
        let disabled_tools = self
            .live(&id)
            .map(|live| live.service.disabled_tools.clone())
            .unwrap_or_default();
        self.services.retain(|live| live.service.id != id);
        let mut live = Live::new(McpService {
            id: id.clone(),
            name: display,
            address: checked.url.clone(),
            phase: McpPhase::Connecting,
            message: McpPhase::Connecting.label().to_owned(),
            disabled_tools,
            ..McpService::default()
        });
        live.named_by_person = named_by_person;
        self.services.push(live);
        self.publish(&id).await;
        tracing::info!(service_id = %id, "connecting service");
        self.establish(&id, None, false, false).await;
    }

    async fn establish(
        &mut self,
        id: &str,
        bearer: Option<String>,
        authorized: bool,
        retried: bool,
    ) {
        let Some(address) = self.address_of(id) else {
            return;
        };
        let request_id = self.rpc_id();
        let body = tools_list_request(request_id, PROTOCOL_MODERN);
        let reply = match self
            .mcp_post(
                &address,
                PROTOCOL_MODERN,
                None,
                bearer.as_deref(),
                &address,
                &body,
            )
            .await
        {
            Ok(reply) => reply,
            Err(failure) => return self.fail(id, failure).await,
        };
        self.route(id, PROTOCOL_MODERN, reply, bearer, authorized, retried)
            .await;
    }

    async fn route(
        &mut self,
        id: &str,
        requested: &str,
        reply: Reply,
        bearer: Option<String>,
        authorized: bool,
        retried: bool,
    ) {
        if reply.status == 401 {
            if authorized {
                if self.live(id).is_some_and(|live| live.uses_static) {
                    self.fail(id, ConnectFailure::NeedsSecret).await;
                } else {
                    // A saved sign-in expired: offer the Sign in button.
                    self.mark(id, McpPhase::NeedsSignIn, ConnectFailure::SignIn.text())
                        .await;
                }
            } else {
                self.authenticate(id, reply.www_authenticate).await;
            }
            return;
        }
        let outcome = interpret_initialize(requested, reply.status, &reply.body);
        match outcome {
            InitializeOutcome::Modern {
                name,
                tools: Some(tools),
            } => {
                self.store_catalogue(id, PROTOCOL_MODERN, None, name, tools)
                    .await;
            }
            InitializeOutcome::Modern { name, tools: None } => {
                self.finish(id, PROTOCOL_MODERN, None, bearer, name).await;
            }
            InitializeOutcome::Legacy { name } => {
                self.finish(id, PROTOCOL_LEGACY, reply.session, bearer, name)
                    .await;
            }
            InitializeOutcome::Retry if requested == PROTOCOL_MODERN && !retried => {
                Box::pin(self.establish(id, bearer, authorized, true)).await;
            }
            InitializeOutcome::FallBack if requested == PROTOCOL_MODERN => {
                let Some(address) = self.address_of(id) else {
                    return;
                };
                let request_id = self.rpc_id();
                let body = initialize_request(request_id, PROTOCOL_LEGACY);
                let legacy = match self
                    .mcp_post(
                        &address,
                        PROTOCOL_LEGACY,
                        None,
                        bearer.as_deref(),
                        &address,
                        &body,
                    )
                    .await
                {
                    Ok(reply) => reply,
                    Err(failure) => return self.fail(id, failure).await,
                };
                Box::pin(self.route(id, PROTOCOL_LEGACY, legacy, bearer, authorized, true)).await;
            }
            InitializeOutcome::FallBack | InitializeOutcome::Retry | InitializeOutcome::Failed => {
                self.fail(id, ConnectFailure::Unreachable).await;
            }
        }
    }

    async fn authenticate(&mut self, id: &str, www_authenticate: String) {
        let Some(address) = self.address_of(id) else {
            return;
        };
        let discovered = match self.discover(&address, &www_authenticate).await {
            Ok(discovered) => discovered,
            Err(failure) => return self.fail(id, failure).await,
        };
        let Some(issuer) = discovered
            .as_ref()
            .and_then(|resource| resource.authorization_servers.first())
            .cloned()
        else {
            return self.use_static_secret(id).await;
        };
        let Some(metadata_url) = authorization_server_metadata_url(&issuer) else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        let metadata = match self.get_text(&metadata_url).await {
            Ok(body) => body,
            Err(failure) => return self.fail(id, failure).await,
        };
        let Some(server) = parse_authorization_server(&metadata) else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        if let Some(live) = self.live_mut(id) {
            live.issuer.clone_from(&server.issuer);
            live.auth = Some(server);
        }
        self.continue_registration(id).await;
    }

    async fn use_static_secret(&mut self, id: &str) {
        let Some(address) = self.address_of(id) else {
            return;
        };
        if let Some(secret) = self.read_secret("", &address, SecretKind::Static).await {
            if let Some(live) = self.live_mut(id) {
                live.uses_static = true;
                live.issuer.clear();
            }
            Box::pin(self.establish(id, Some(secret), true, false)).await;
            return;
        }
        self.mark(id, McpPhase::Failed, ConnectFailure::NeedsSecret.text())
            .await;
    }

    async fn continue_registration(&mut self, id: &str) {
        let Some(server) = self.auth_of(id) else {
            self.fail(id, ConnectFailure::SignIn).await;
            return;
        };
        let saved = self
            .read_secret(&server.issuer, "", SecretKind::Client)
            .await
            .and_then(|raw| serde_json::from_str::<SavedClient>(&raw).ok());
        let metadata = self
            .read_secret(&server.issuer, "", SecretKind::Metadata)
            .await;
        let choice = choose_registration(&RegistrationOffer {
            saved_client_id: saved.as_ref().map(|client| client.client_id.as_str()),
            metadata_url: metadata.as_deref(),
            server_supports_metadata: server.supports_client_metadata,
            registration_endpoint: server.registration_endpoint.as_deref(),
        });
        match choice {
            RegistrationChoice::AskPerson => {
                self.mark(
                    id,
                    McpPhase::NeedsClientDetails,
                    ConnectFailure::NeedsClientDetails.text(),
                )
                .await;
            }
            RegistrationChoice::Saved { client_id } => {
                let secret = saved.and_then(|client| client.client_secret);
                self.begin_sign_in(
                    id,
                    ClientMaterial {
                        client_id,
                        client_secret: secret,
                    },
                )
                .await;
            }
            RegistrationChoice::ClientMetadata { url } => {
                self.begin_sign_in(
                    id,
                    ClientMaterial {
                        client_id: url,
                        client_secret: None,
                    },
                )
                .await;
            }
            RegistrationChoice::Dynamic { endpoint } => {
                self.register_dynamically(id, endpoint).await;
            }
        }
    }

    async fn register_dynamically(&mut self, id: &str, endpoint: String) {
        let Ok(listener) = TcpListener::bind("127.0.0.1:0").await else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        let port = match listener.local_addr() {
            Ok(address) => address.port(),
            Err(_) => return self.fail(id, ConnectFailure::SignIn).await,
        };
        let redirect = format!("http://127.0.0.1:{port}/callback");
        let document = dynamic_registration_document(&redirect);
        let reply = match self.post_json(&endpoint, &document).await {
            Ok(reply) => reply,
            Err(failure) => return self.fail(id, failure).await,
        };
        let Some(material) = registered_client(reply.status, &reply.body) else {
            self.mark(
                id,
                McpPhase::NeedsClientDetails,
                ConnectFailure::NeedsClientDetails.text(),
            )
            .await;
            return;
        };
        if let Some(issuer) = self.issuer_of(id) {
            self.store_client(&issuer, &material).await;
        }
        self.finish_sign_in(id, listener, redirect, material).await;
    }

    async fn begin_sign_in(&mut self, id: &str, material: ClientMaterial) {
        let Ok(listener) = TcpListener::bind("127.0.0.1:0").await else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        let port = match listener.local_addr() {
            Ok(address) => address.port(),
            Err(_) => return self.fail(id, ConnectFailure::SignIn).await,
        };
        let redirect = format!("http://127.0.0.1:{port}/callback");
        self.finish_sign_in(id, listener, redirect, material).await;
    }

    async fn finish_sign_in(
        &mut self,
        id: &str,
        listener: TcpListener,
        redirect: String,
        material: ClientMaterial,
    ) {
        let Some(server) = self.auth_of(id) else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        let Some(resource) = self.address_of(id) else {
            return;
        };
        let proof = SignInProof::generate();
        let challenge = s256_challenge(proof.verifier());
        let Ok(mut authorize) = Url::parse(&server.authorization_endpoint) else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        {
            let mut pairs = authorize.query_pairs_mut();
            pairs.append_pair("response_type", "code");
            pairs.append_pair("client_id", &material.client_id);
            pairs.append_pair("redirect_uri", &redirect);
            pairs.append_pair("code_challenge", &challenge);
            pairs.append_pair("code_challenge_method", "S256");
            pairs.append_pair("state", proof.state());
            pairs.append_pair("resource", &resource);
        }
        let auth_url = authorize.to_string();
        self.mark(id, McpPhase::NeedsSignIn, ConnectFailure::SignIn.text())
            .await;
        self.emit(McpEvent::OpenSignIn {
            service_id: id.to_owned(),
            url: auth_url.clone(),
        })
        .await;
        let Some(inbox) = self.inbox.upgrade() else {
            return;
        };
        let expected_state = proof.state().to_owned();
        let pending = Box::new(PendingSignIn {
            server,
            material,
            redirect,
            resource,
            proof,
        });
        let http = self.http.clone();
        let service_id = id.to_owned();
        // Wait in a task of its own: the browser can take minutes, and other
        // services must keep working meanwhile.
        let waiter = self.handle.spawn(async move {
            // A service that redirects straight back (no page to show) finishes at once.
            let driven = get_reply(&http, &auth_url).await.ok().and_then(|reply| {
                callback_from_redirect(reply.location.as_deref(), &pending.redirect)
            });
            let outcome = match driven {
                Some(callback) => Ok(callback),
                None => tokio::time::timeout(
                    SIGN_IN_TIMEOUT,
                    wait_for_sign_in(listener, expected_state),
                )
                .await
                .map_err(|_| ConnectFailure::SignIn)
                .and_then(|result| {
                    result.map_err(|text| {
                        if text == ConnectFailure::SignInMismatch.text() {
                            ConnectFailure::SignInMismatch
                        } else {
                            ConnectFailure::SignIn
                        }
                    })
                }),
            };
            let _ = inbox
                .send(Input::SignedIn {
                    id: service_id,
                    pending,
                    outcome,
                })
                .await;
        });
        if let Some(previous) = self.listeners.insert(id.to_owned(), waiter) {
            previous.abort();
        }
    }

    async fn complete_sign_in(
        &mut self,
        id: String,
        pending: PendingSignIn,
        outcome: Result<SignInCallback, ConnectFailure>,
    ) {
        self.listeners.remove(&id);
        if self.live(&id).is_none_or(|live| !live.service.enabled) {
            return;
        }
        let callback = match outcome {
            Ok(callback) => callback,
            Err(failure) => return self.fail(&id, failure).await,
        };
        let issuer = pending.server.issuer.clone();
        if !constant_time_str_eq(&callback.state, pending.proof.state())
            || !authorization_issuer_ok(&issuer, callback.issuer.as_deref())
        {
            return self.fail(&id, ConnectFailure::SignInMismatch).await;
        }
        let Some(token) = self
            .exchange_code(
                &pending.server,
                &pending.material,
                &pending.redirect,
                &pending.resource,
                &pending.proof,
                &callback,
            )
            .await
        else {
            return self.fail(&id, ConnectFailure::SignIn).await;
        };
        self.store_secret(&issuer, &pending.resource, SecretKind::Access, &token)
            .await;
        if let Some(live) = self.live_mut(&id) {
            live.issuer.clone_from(&issuer);
            live.uses_static = false;
        }
        Box::pin(self.establish(&id, Some(token), true, false)).await;
    }

    async fn exchange_code(
        &self,
        server: &AuthorizationServer,
        material: &ClientMaterial,
        redirect: &str,
        resource: &str,
        proof: &SignInProof,
        callback: &SignInCallback,
    ) -> Option<String> {
        let mut pairs = vec![
            ("grant_type", "authorization_code"),
            ("code", callback.code.as_str()),
            ("redirect_uri", redirect),
            ("client_id", material.client_id.as_str()),
            ("code_verifier", proof.verifier()),
            ("resource", resource),
        ];
        if let Some(secret) = material.client_secret.as_deref() {
            pairs.push(("client_secret", secret));
        }
        let body = form_body(&pairs);
        let reply = self
            .http
            .post(&server.token_endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await
            .ok()?;
        let status = reply.status().as_u16();
        let text = read_limited(reply).await.ok()?;
        if !(200..300).contains(&status) {
            return None;
        }
        let value = json_from_body(&text)?;
        let returned = value.get("iss").and_then(Value::as_str);
        if !authorization_issuer_ok(&server.issuer, returned) {
            return None;
        }
        value
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
    }

    async fn finish(
        &mut self,
        id: &str,
        protocol: &str,
        session: Option<String>,
        bearer: Option<String>,
        server_name: Option<String>,
    ) {
        let Some(address) = self.address_of(id) else {
            return;
        };
        let session_header = (protocol == PROTOCOL_LEGACY)
            .then_some(session.as_deref())
            .flatten();
        let _ = self
            .mcp_post(
                &address,
                protocol,
                session_header,
                bearer.as_deref(),
                &address,
                &initialized_notification(),
            )
            .await;
        let request_id = self.rpc_id();
        let body = tools_list_request(request_id, protocol);
        let listed = match self
            .mcp_post(
                &address,
                protocol,
                session_header,
                bearer.as_deref(),
                &address,
                &body,
            )
            .await
        {
            Ok(reply) => reply,
            Err(failure) => return self.fail(id, failure).await,
        };
        if listed.status == 401 {
            return self.fail(id, ConnectFailure::SignIn).await;
        }
        let parsed = json_from_body(&listed.body);
        let Some(result) = parsed.as_ref().and_then(rpc_result) else {
            return self.fail(id, ConnectFailure::Unreachable).await;
        };
        let tools = parse_tool_list(result);
        self.store_catalogue(id, protocol, session, server_name, tools)
            .await;
    }

    async fn store_catalogue(
        &mut self,
        id: &str,
        protocol: &str,
        session: Option<String>,
        server_name: Option<String>,
        tools: Vec<sovra_core::mcp::McpToolInfo>,
    ) {
        let Some(service) = self.connected_service(id, protocol, session, server_name, tools)
        else {
            return;
        };
        let tool_count = service.tools.len();
        self.sync_board();
        self.save_catalogue().await;
        tracing::info!(service_id = %id, tools = tool_count, "service connected");
        self.emit(McpEvent::Upsert(service)).await;
    }

    fn connected_service(
        &mut self,
        id: &str,
        protocol: &str,
        session: Option<String>,
        server_name: Option<String>,
        tools: Vec<sovra_core::mcp::McpToolInfo>,
    ) -> Option<McpService> {
        let live = self.live_mut(id)?;
        if !live.named_by_person
            && let Some(name) = server_name.filter(|name| !name.is_empty())
        {
            live.service.name = name;
        }
        protocol.clone_into(&mut live.protocol);
        live.session = if protocol == PROTOCOL_LEGACY {
            session
        } else {
            None
        };
        live.service.tools = tools;
        live.service.phase = McpPhase::Connected;
        "Connected.".clone_into(&mut live.service.message);
        Some(live.service.clone())
    }

    async fn submit_client(
        &mut self,
        id: String,
        client_id: String,
        client_secret: Option<String>,
    ) {
        let client_id = client_id.trim().to_owned();
        if client_id.is_empty() {
            self.mark(
                &id,
                McpPhase::NeedsClientDetails,
                ConnectFailure::NeedsClientDetails.text(),
            )
            .await;
            return;
        }
        let Some(issuer) = self.issuer_of(&id) else {
            self.fail(&id, ConnectFailure::NeedsClientDetails).await;
            return;
        };
        let material = ClientMaterial {
            client_id,
            client_secret: client_secret
                .as_deref()
                .map(str::trim)
                .filter(|secret| !secret.is_empty())
                .map(str::to_owned),
        };
        self.store_client(&issuer, &material).await;
        self.begin_sign_in(&id, material).await;
    }

    async fn sign_in(&mut self, id: String) {
        if self.auth_of(&id).is_none() {
            Box::pin(self.establish(&id, None, false, false)).await;
            return;
        }
        self.continue_registration(&id).await;
    }

    async fn remove(&mut self, id: String) {
        let Some(position) = self.services.iter().position(|live| live.service.id == id) else {
            self.emit(McpEvent::Removed { id }).await;
            return;
        };
        let live = self.services.remove(position);
        self.programs.remove(&id);
        if let Some(listener) = self.listeners.remove(&id) {
            listener.abort();
        }
        let access = secret_storage_id(&live.issuer, &live.service.address, SecretKind::Access);
        let static_secret = secret_storage_id("", &live.service.address, SecretKind::Static);
        let settings = secret_storage_id("", &id, SecretKind::Settings);
        self.vault.delete(&access).await;
        self.vault.delete(&static_secret).await;
        self.vault.delete(&settings).await;
        self.sync_board();
        self.save_catalogue().await;
        tracing::info!(service_id = %id, "service removed");
        self.emit(McpEvent::Removed { id }).await;
    }

    /// Starts one tool call and returns at once; the answer arrives from a task of
    /// its own, so a slow service never holds up the others.
    async fn perform_call(&mut self, request: RemoteCall, reply: Option<CallReply>) {
        let allowed = self.live(&request.service_id).is_some_and(|live| {
            live.service.enabled
                && live.service.phase == McpPhase::Connected
                && live.service.tool_is_on(&request.tool)
        });
        let Some(snap) = self.snapshot(&request.service_id).filter(|_| allowed) else {
            let outcome = self.refuse(&request).await;
            if let Some(reply) = reply {
                let _ = reply.send(outcome);
            }
            return;
        };
        let arguments = if request.arguments.is_null() {
            json!({})
        } else {
            request.arguments.clone()
        };
        let events = self.events.clone();
        let session_id = Some(request.session_id.clone()).filter(|id| !id.is_empty());
        let tool = request.tool.clone();
        if let Some(program) = self.programs.get(&request.service_id) {
            let (answer, answered) = tokio::sync::oneshot::channel();
            program.call(request.tool, arguments, answer);
            drop(self.handle.spawn(async move {
                let text = answered
                    .await
                    .unwrap_or_else(|_| TOOL_FAILED_TEXT.to_owned());
                finish_call(&events, snap, tool, text, session_id, reply).await;
            }));
            return;
        }
        let bearer = if snap.uses_static {
            self.read_secret("", &snap.address, SecretKind::Static)
                .await
        } else if snap.issuer.is_empty() {
            None
        } else {
            self.read_secret(&snap.issuer, &snap.address, SecretKind::Access)
                .await
        };
        let headers = self.headers_for(&snap.address);
        let request_id = self.rpc_id();
        let body = tools_call_request(request_id, &snap.protocol, &request.tool, &arguments);
        let http = self.http.clone();
        drop(self.handle.spawn(async move {
            let session = (snap.protocol == PROTOCOL_LEGACY)
                .then_some(snap.session.as_deref())
                .flatten();
            let reply_text = match post_mcp(
                &http,
                &snap.address,
                &snap.protocol,
                session,
                bearer.as_deref(),
                &snap.address,
                &body,
                &headers,
            )
            .await
            {
                Ok(reply) if (200..300).contains(&reply.status) => json_from_body(&reply.body)
                    .as_ref()
                    .and_then(rpc_result)
                    .map(parse_tool_text)
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| TOOL_FAILED_TEXT.to_owned()),
                Ok(reply) if reply.status == 401 => ConnectFailure::SignIn.text().to_owned(),
                _ => TOOL_FAILED_TEXT.to_owned(),
            };
            finish_call(&events, snap, tool, reply_text, session_id, reply).await;
        }));
    }

    fn headers_for(&self, address: &str) -> Vec<(String, String)> {
        self.services
            .iter()
            .find(|live| live.service.address == address && !live.service.transport.is_program())
            .map(|live| live.headers.clone())
            .unwrap_or_default()
    }

    async fn refuse(&mut self, request: &RemoteCall) -> Result<RemoteOutput, String> {
        let name = self
            .live(&request.service_id)
            .map_or_else(|| "Service".to_owned(), |live| live.service.name.clone());
        let text = ConnectFailure::ToolUnavailable.text().to_owned();
        self.emit(McpEvent::ToolResult {
            service_id: request.service_id.clone(),
            service_name: name.clone(),
            tool: request.tool.clone(),
            text: text.clone(),
            session_id: Some(request.session_id.clone()).filter(|id| !id.is_empty()),
        })
        .await;
        Ok(RemoteOutput {
            service_name: name,
            text,
        })
    }

    async fn discover(
        &self,
        address: &str,
        www_authenticate: &str,
    ) -> Result<Option<sovra_core::mcp::ProtectedResource>, ConnectFailure> {
        if let Some(url) = resource_metadata_parameter(www_authenticate) {
            let body = self.get_text(&url).await?;
            return parse_protected_resource(&body)
                .map(Some)
                .ok_or(ConnectFailure::SignIn);
        }
        for url in protected_resource_well_known_urls(address) {
            if let Ok(body) = self.get_text(&url).await
                && let Some(resource) = parse_protected_resource(&body)
            {
                return Ok(Some(resource));
            }
        }
        Ok(None)
    }

    async fn mcp_post(
        &self,
        url: &str,
        protocol: &str,
        session: Option<&str>,
        bearer: Option<&str>,
        bound_server: &str,
        body: &Value,
    ) -> Result<Reply, ConnectFailure> {
        let headers = self.headers_for(bound_server);
        post_mcp(
            &self.http,
            url,
            protocol,
            session,
            bearer,
            bound_server,
            body,
            &headers,
        )
        .await
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<Reply, ConnectFailure> {
        let payload = serde_json::to_string(body).map_err(|_| ConnectFailure::Unreachable)?;
        let response = self
            .http
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|_| ConnectFailure::Unreachable)?;
        reply_from(response).await
    }

    async fn get_text(&self, url: &str) -> Result<String, ConnectFailure> {
        let reply = self.get_reply(url).await?;
        if (200..300).contains(&reply.status) {
            Ok(reply.body)
        } else {
            Err(ConnectFailure::SignIn)
        }
    }

    async fn get_reply(&self, url: &str) -> Result<Reply, ConnectFailure> {
        get_reply(&self.http, url).await
    }

    fn rpc_id(&mut self) -> u64 {
        let id = self.next_rpc;
        self.next_rpc = self.next_rpc.saturating_add(1);
        id
    }

    fn address_of(&self, id: &str) -> Option<String> {
        self.live(id).map(|live| live.service.address.clone())
    }

    fn issuer_of(&self, id: &str) -> Option<String> {
        self.live(id)
            .map(|live| live.issuer.clone())
            .filter(|issuer| !issuer.is_empty())
    }

    fn auth_of(&self, id: &str) -> Option<AuthorizationServer> {
        self.live(id).and_then(|live| live.auth.clone())
    }

    fn live(&self, id: &str) -> Option<&Live> {
        self.services.iter().find(|live| live.service.id == id)
    }

    fn live_mut(&mut self, id: &str) -> Option<&mut Live> {
        self.services.iter_mut().find(|live| live.service.id == id)
    }

    fn snapshot(&self, id: &str) -> Option<Snap> {
        self.live(id).map(|live| Snap {
            id: live.service.id.clone(),
            name: live.service.name.clone(),
            address: live.service.address.clone(),
            protocol: live.protocol.clone(),
            session: live.session.clone(),
            issuer: live.issuer.clone(),
            uses_static: live.uses_static,
        })
    }

    async fn read_secret(&self, issuer: &str, server: &str, kind: SecretKind) -> Option<String> {
        let id = secret_storage_id(issuer, server, kind);
        let raw = self.vault.get(&id).await?;
        let record: SecretRecord = serde_json::from_str(&raw).ok()?;
        accept_secret(&record, issuer, server, kind).map(str::to_owned)
    }

    async fn store_secret(&mut self, issuer: &str, server: &str, kind: SecretKind, value: &str) {
        let record = SecretRecord::new(issuer, server, kind, value);
        let Ok(encoded) = serde_json::to_string(&record) else {
            return;
        };
        let id = secret_storage_id(issuer, server, kind);
        self.vault.put(&id, &encoded).await;
    }

    async fn store_client(&mut self, issuer: &str, material: &ClientMaterial) {
        let saved = json!({
            "client_id": material.client_id,
            "client_secret": material.client_secret,
        });
        self.store_secret(issuer, "", SecretKind::Client, &saved.to_string())
            .await;
    }

    async fn mark(&mut self, id: &str, phase: McpPhase, message: &str) {
        let Some(live) = self.live_mut(id) else {
            return;
        };
        live.service.phase = phase;
        message.clone_into(&mut live.service.message);
        if phase != McpPhase::Connected {
            live.service.tools.clear();
        }
        self.sync_board();
        self.save_catalogue().await;
        let service = self.live(id).expect("service still exists").service.clone();
        self.emit(McpEvent::Upsert(service)).await;
    }

    async fn fail(&mut self, id: &str, failure: ConnectFailure) {
        self.mark(id, McpPhase::Failed, failure.text()).await;
    }

    async fn publish(&mut self, id: &str) {
        self.sync_board();
        self.save_catalogue().await;
        if let Some(service) = self.live(id).map(|live| live.service.clone()) {
            self.emit(McpEvent::Upsert(service)).await;
        }
    }

    fn sync_board(&self) {
        let offers = self
            .services
            .iter()
            .filter(|live| live.service.enabled && live.service.phase == McpPhase::Connected)
            .flat_map(|live| {
                live.service
                    .tools
                    .iter()
                    .filter(|tool| live.service.tool_is_on(&tool.name))
                    .map(|tool| ToolOffer {
                        service_id: live.service.id.clone(),
                        service_name: live.service.name.clone(),
                        tool: tool.clone(),
                    })
            })
            .collect();
        self.board.replace(offers);
    }

    fn view_state(&self) -> McpState {
        McpState {
            services: self
                .services
                .iter()
                .map(|live| live.service.clone())
                .collect(),
            notice: None,
        }
    }

    async fn save_catalogue(&mut self) {
        let entries: Vec<CatalogueEntry> = self
            .services
            .iter()
            .map(|live| CatalogueEntry {
                id: live.service.id.clone(),
                name: live.service.name.clone(),
                address: live.service.address.clone(),
                issuer: live.issuer.clone(),
                transport: live.service.transport.clone(),
                enabled: live.service.enabled,
                disabled_tools: live.service.disabled_tools.clone(),
                header_names: live.service.header_names.clone(),
                approved: live.approved.clone(),
            })
            .collect();
        if self.catalogue_path.is_some() {
            self.write_catalogue_file(&entries).await;
        } else if let Ok(encoded) = serde_json::to_string(&entries) {
            self.vault.put(CATALOGUE_ID, &encoded).await;
        }
    }

    /// Write-then-rename so a crash never leaves half a list.
    async fn write_catalogue_file(&self, entries: &[CatalogueEntry]) -> bool {
        let Some(path) = &self.catalogue_path else {
            return false;
        };
        let file = CatalogueFile {
            version: CATALOGUE_VERSION,
            services: Vec::new(),
        };
        let Ok(mut value) = serde_json::to_value(&file) else {
            return false;
        };
        value["services"] = serde_json::to_value(entries).unwrap_or_default();
        let Ok(bytes) = serde_json::to_vec_pretty(&value) else {
            return false;
        };
        let temporary = path.with_extension("json.tmp");
        let written = async {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&temporary, bytes).await?;
            tokio::fs::rename(&temporary, path).await
        }
        .await;
        if let Err(error) = &written {
            tracing::warn!(%error, "could not save connected services list");
        }
        written.is_ok()
    }

    async fn emit(&self, event: McpEvent) {
        let _ = self.events.send(Event::Mcp(event)).await;
    }
}

struct Snap {
    id: String,
    name: String,
    address: String,
    protocol: String,
    session: Option<String>,
    issuer: String,
    uses_static: bool,
}

const FULL_TEXT: &str =
    "You already have the most services Sovra can keep. Remove one you don't use first.";

fn label_for(name: &str, address: &str) -> String {
    if name.trim().is_empty() {
        address.to_owned()
    } else {
        name.trim().to_owned()
    }
}

/// Emits the tool result to the chat and answers the waiting model turn.
async fn finish_call(
    events: &Sender<Event>,
    snap: Snap,
    tool: String,
    text: String,
    session_id: Option<String>,
    reply: Option<CallReply>,
) {
    let output = RemoteOutput {
        service_name: snap.name.clone(),
        text: text.clone(),
    };
    let _ = events
        .send(Event::Mcp(McpEvent::ToolResult {
            service_id: snap.id,
            service_name: snap.name,
            tool,
            text,
            session_id,
        }))
        .await;
    if let Some(reply) = reply {
        let _ = reply.send(Ok(output));
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one MCP request: transport fields, credentials and the body"
)]
async fn post_mcp(
    http: &reqwest::Client,
    url: &str,
    protocol: &str,
    session: Option<&str>,
    bearer: Option<&str>,
    bound_server: &str,
    body: &Value,
    headers: &[(String, String)],
) -> Result<Reply, ConnectFailure> {
    let Some(origin) = request_origin(url) else {
        return Err(ConnectFailure::Unreachable);
    };
    let payload = serde_json::to_string(body).map_err(|_| ConnectFailure::Unreachable)?;
    let mut request = http
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", ACCEPT_HEADER)
        .header(PROTOCOL_HEADER, protocol)
        .header("Origin", origin)
        .body(payload);
    if protocol == PROTOCOL_MODERN
        && let Some(method) = body.get("method").and_then(Value::as_str)
    {
        request = request.header(METHOD_HEADER, method);
        if let Some(name) = mirrored_name(body) {
            request = request.header(NAME_HEADER, header_field_value(name));
        }
    }
    if protocol == PROTOCOL_LEGACY
        && let Some(session) = session.filter(|session| !session.is_empty())
    {
        request = request.header("Mcp-Session-Id", session);
    }
    // Saved headers and tokens go only to the exact address they were saved for.
    let bound = token_may_be_sent(bound_server, url);
    let mut sets_authorization = false;
    if bound {
        for (name, value) in headers
            .iter()
            .filter(|(name, value)| header_allowed(name, value))
        {
            sets_authorization |= name.eq_ignore_ascii_case("authorization");
            request = request.header(name.as_str(), value.as_str());
        }
    }
    if let Some(token) = bearer.filter(|_| bound && !sets_authorization) {
        let mut header = String::from("Bearer ");
        header.push_str(token);
        request = request.header("Authorization", header);
    }
    let response = request
        .send()
        .await
        .map_err(|_| ConnectFailure::Unreachable)?;
    reply_from(response).await
}

async fn get_reply(http: &reqwest::Client, url: &str) -> Result<Reply, ConnectFailure> {
    let response = http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| ConnectFailure::Unreachable)?;
    reply_from(response).await
}

fn registered_client(status: u16, body: &str) -> Option<ClientMaterial> {
    if !(200..300).contains(&status) {
        return None;
    }
    let value = json_from_body(body)?;
    let client_id = value.get("client_id")?.as_str()?.trim();
    if client_id.is_empty() {
        return None;
    }
    let client_secret = value
        .get("client_secret")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(str::to_owned);
    Some(ClientMaterial {
        client_id: client_id.to_owned(),
        client_secret,
    })
}

fn callback_from_redirect(location: Option<&str>, redirect: &str) -> Option<SignInCallback> {
    let location = location?;
    let rest = location.strip_prefix(redirect)?;
    let query = rest.strip_prefix('?').unwrap_or("");
    if !rest.is_empty() && !rest.starts_with('?') {
        return None;
    }
    parse_sign_in_callback(&format!("GET /callback?{query} HTTP/1.1"))
}

fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in pairs {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

async fn reply_from(response: reqwest::Response) -> Result<Reply, ConnectFailure> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = read_limited(response).await?;
    Ok(Reply {
        status,
        session: header_text(&headers, "mcp-session-id"),
        www_authenticate: header_text(&headers, "www-authenticate").unwrap_or_default(),
        location: header_text(&headers, "location"),
        body,
    })
}

async fn read_limited(response: reqwest::Response) -> Result<String, ConnectFailure> {
    let bytes = response
        .bytes()
        .await
        .map_err(|_| ConnectFailure::Unreachable)?;
    if bytes.len() > BODY_LIMIT {
        return Err(ConnectFailure::Unreachable);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn header_text(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

async fn wait_for_sign_in(
    listener: TcpListener,
    expected_state: String,
) -> Result<SignInCallback, String> {
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| ConnectFailure::SignIn.text().to_owned())?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let mut buffer = vec![0_u8; LOOPBACK_LIMIT];
        let read = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut buffer)).await;
        let Ok(Ok(length)) = read else { continue };
        let request = String::from_utf8_lossy(&buffer[..length]);
        let line = request.lines().next().unwrap_or_default();
        if !line.contains("/callback") {
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }
        let Some(callback) = parse_sign_in_callback(line) else {
            let _ = stream.write_all(callback_page(false).as_bytes()).await;
            return Err(ConnectFailure::SignInMismatch.text().to_owned());
        };
        let matches = constant_time_str_eq(&callback.state, &expected_state);
        let _ = stream.write_all(callback_page(matches).as_bytes()).await;
        if matches {
            return Ok(callback);
        }
        return Err(ConnectFailure::SignInMismatch.text().to_owned());
    }
}

#[cfg(test)]
mod tests;
