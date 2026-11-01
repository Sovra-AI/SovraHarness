//! Connects a service by its web address and calls one of its tools.
//!
//! The screen only sends [`sovra_core::mcp::McpCommand`]. This task owns HTTP,
//! the loopback sign-in listener, and the credential store. Tokens and service
//! secrets are never written to events or logs.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_channel::{Receiver, Sender};
use serde_json::{Value, json};
use sovra_core::Event;
use sovra_core::auth::{callback_page, constant_time_str_eq};
use sovra_core::mcp::{
    ACCEPT_HEADER, AuthorizationServer, ConnectFailure, InitializeOutcome, METHOD_HEADER,
    McpCommand, McpEvent, McpPhase, McpService, McpState, NAME_HEADER, PROTOCOL_HEADER,
    PROTOCOL_LEGACY, PROTOCOL_MODERN, RegistrationChoice, RegistrationOffer, RemoteCall,
    RemoteOutput, RemoteToolCaller, SecretKind, SecretRecord, SignInCallback, TOOL_FAILED_TEXT,
    ToolOffer, accept_secret, authorization_issuer_ok, authorization_server_metadata_url,
    check_service_address, choose_registration, dynamic_registration_document, header_field_value,
    initialize_request, initialized_notification, interpret_initialize, json_from_body,
    mirrored_name, parse_authorization_server, parse_protected_resource, parse_sign_in_callback,
    parse_tool_list, parse_tool_text, protected_resource_well_known_urls, request_origin,
    resource_metadata_parameter, rpc_result, s256_challenge, secret_storage_id, service_id_for,
    token_may_be_sent, tools_call_request, tools_list_request,
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

enum Input {
    Command(McpCommand),
    Invoke {
        request: RemoteCall,
        reply: tokio::sync::oneshot::Sender<Result<RemoteOutput, String>>,
    },
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
            events,
            http: http_client(),
            vault: Vault::new(repository),
            services: Vec::new(),
            board: Arc::clone(&board),
            next_rpc: 1,
            listener: None,
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
    events: Sender<Event>,
    http: reqwest::Client,
    vault: Vault,
    services: Vec<Live>,
    board: Arc<ToolBoard>,
    next_rpc: u64,
    listener: Option<tokio::task::JoinHandle<()>>,
}

struct Live {
    service: McpService,
    protocol: String,
    session: Option<String>,
    issuer: String,
    auth: Option<AuthorizationServer>,
    named_by_person: bool,
    uses_static: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CatalogueEntry {
    id: String,
    name: String,
    address: String,
    issuer: String,
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
                Input::Invoke { request, reply } => {
                    let outcome = self.perform_call(request).await;
                    let _ = reply.send(outcome);
                }
            }
        }
        self.stop_listener();
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
                if let Ok(output) = self.perform_call(request).await {
                    tracing::debug!(service = %output.service_name.len(), "service action finished");
                }
            }
        }
    }

    async fn load(&mut self) {
        let raw = self.vault.get(CATALOGUE_ID).await.unwrap_or_default();
        let entries: Vec<CatalogueEntry> = serde_json::from_str(&raw).unwrap_or_default();
        self.services = entries
            .into_iter()
            .map(|entry| Live {
                service: McpService {
                    id: entry.id,
                    name: entry.name,
                    address: entry.address,
                    phase: McpPhase::Saved,
                    message: McpPhase::Saved.label().to_owned(),
                    tools: Vec::new(),
                },
                protocol: PROTOCOL_MODERN.to_owned(),
                session: None,
                issuer: entry.issuer,
                auth: None,
                named_by_person: true,
                uses_static: false,
            })
            .collect();
        self.sync_board();
        self.emit(McpEvent::Loaded(self.view_state())).await;
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
        self.services.retain(|live| live.service.id != id);
        self.services.push(Live {
            service: McpService {
                id: id.clone(),
                name: display,
                address: checked.url.clone(),
                phase: McpPhase::Connecting,
                message: McpPhase::Connecting.label().to_owned(),
                tools: Vec::new(),
            },
            protocol: PROTOCOL_MODERN.to_owned(),
            session: None,
            issuer: String::new(),
            auth: None,
            named_by_person,
            uses_static: false,
        });
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
                let failure = if self.live(id).is_some_and(|live| live.uses_static) {
                    ConnectFailure::NeedsSecret
                } else {
                    ConnectFailure::SignIn
                };
                self.fail(id, failure).await;
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
        let issuer = server.issuer.clone();
        self.mark(id, McpPhase::NeedsSignIn, ConnectFailure::SignIn.text())
            .await;
        self.emit(McpEvent::OpenSignIn {
            service_id: id.to_owned(),
            url: auth_url.clone(),
        })
        .await;
        let expected_state = proof.state().to_owned();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.stop_listener();
        self.listener = Some(self.handle.spawn(async move {
            let outcome =
                tokio::time::timeout(SIGN_IN_TIMEOUT, wait_for_sign_in(listener, expected_state))
                    .await
                    .map_err(|_| ConnectFailure::SignIn.text().to_owned())
                    .and_then(|result| result);
            let _ = tx.send(outcome);
        }));
        let driven = self
            .get_reply(&auth_url)
            .await
            .ok()
            .and_then(|reply| callback_from_redirect(reply.location.as_deref(), &redirect));
        let callback = if let Some(callback) = driven {
            self.stop_listener();
            callback
        } else {
            match rx.await {
                Ok(Ok(callback)) => callback,
                Ok(Err(_)) | Err(_) => {
                    return self.fail(id, ConnectFailure::SignIn).await;
                }
            }
        };
        if !constant_time_str_eq(&callback.state, proof.state())
            || !authorization_issuer_ok(&issuer, callback.issuer.as_deref())
        {
            return self.fail(id, ConnectFailure::SignInMismatch).await;
        }
        let Some(token) = self
            .exchange_code(&server, &material, &redirect, &resource, &proof, &callback)
            .await
        else {
            return self.fail(id, ConnectFailure::SignIn).await;
        };
        self.store_secret(&issuer, &resource, SecretKind::Access, &token)
            .await;
        if let Some(live) = self.live_mut(id) {
            live.issuer.clone_from(&issuer);
            live.uses_static = false;
        }
        Box::pin(self.establish(id, Some(token), true, false)).await;
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
        let access = secret_storage_id(&live.issuer, &live.service.address, SecretKind::Access);
        let static_secret = secret_storage_id("", &live.service.address, SecretKind::Static);
        self.vault.delete(&access).await;
        self.vault.delete(&static_secret).await;
        self.sync_board();
        self.save_catalogue().await;
        tracing::info!(service_id = %id, "service removed");
        self.emit(McpEvent::Removed { id }).await;
    }

    async fn perform_call(&mut self, request: RemoteCall) -> Result<RemoteOutput, String> {
        let Some(snap) = self.snapshot(&request.service_id) else {
            return self.refuse(&request).await;
        };
        if snap.phase != McpPhase::Connected {
            return self.refuse(&request).await;
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
        if self.phase_of(&request.service_id) != Some(McpPhase::Connected) {
            return self.refuse(&request).await;
        }
        let arguments = if request.arguments.is_null() {
            json!({})
        } else {
            request.arguments.clone()
        };
        let session = (snap.protocol == PROTOCOL_LEGACY)
            .then_some(snap.session.as_deref())
            .flatten();
        let request_id = self.rpc_id();
        let body = tools_call_request(request_id, &snap.protocol, &request.tool, &arguments);
        let reply = self
            .mcp_post(
                &snap.address,
                &snap.protocol,
                session,
                bearer.as_deref(),
                &snap.address,
                &body,
            )
            .await;
        let text = match reply {
            Ok(reply) if (200..300).contains(&reply.status) => json_from_body(&reply.body)
                .as_ref()
                .and_then(rpc_result)
                .map(parse_tool_text)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| TOOL_FAILED_TEXT.to_owned()),
            Ok(reply) if reply.status == 401 => ConnectFailure::SignIn.text().to_owned(),
            _ => TOOL_FAILED_TEXT.to_owned(),
        };
        let output = RemoteOutput {
            service_name: snap.name.clone(),
            text: text.clone(),
        };
        self.emit(McpEvent::ToolResult {
            service_id: snap.id,
            service_name: snap.name,
            tool: request.tool,
            text,
            session_id: Some(request.session_id).filter(|id| !id.is_empty()),
        })
        .await;
        Ok(output)
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
        let Some(origin) = request_origin(url) else {
            return Err(ConnectFailure::Unreachable);
        };
        let payload = serde_json::to_string(body).map_err(|_| ConnectFailure::Unreachable)?;
        let mut request = self
            .http
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
        if let Some(token) = bearer.filter(|_| token_may_be_sent(bound_server, url)) {
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
        let response = self
            .http
            .get(url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|_| ConnectFailure::Unreachable)?;
        reply_from(response).await
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

    fn phase_of(&self, id: &str) -> Option<McpPhase> {
        self.live(id).map(|live| live.service.phase)
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
            phase: live.service.phase,
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
            .filter(|live| live.service.phase == McpPhase::Connected)
            .flat_map(|live| {
                live.service.tools.iter().map(|tool| ToolOffer {
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
            })
            .collect();
        if let Ok(encoded) = serde_json::to_string(&entries) {
            self.vault.put(CATALOGUE_ID, &encoded).await;
        }
    }

    async fn emit(&self, event: McpEvent) {
        let _ = self.events.send(Event::Mcp(event)).await;
    }

    fn stop_listener(&mut self) {
        if let Some(task) = self.listener.take() {
            task.abort();
        }
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
    phase: McpPhase,
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
