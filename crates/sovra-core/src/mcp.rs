//! Connected services (MCP servers): web addresses and programs on this computer.
//!
//! Pure rules only: which addresses are allowed, how a sign-in challenge is read,
//! which registration method to use, which saved secret may be reused, and the
//! words a person sees. Managing many services (programs, pasted settings, the
//! review before a program runs, search) is in [`servers`]. HTTP, processes and
//! the browser hand-off live in `sovra-runtime`. Design: `docs/connected-services.md`.

mod servers;

pub use servers::{
    DraftSetup, ImportProblem, MAX_SERVICES, MAX_TOOL_NAME, McpSummary, McpTransport, ProgramNote,
    SAMPLE_SERVER_FLAG, ServerDraft, approval_fingerprint, bounded_tool_alias, env_name_allowed,
    header_allowed, name_for_program, parse_server_settings, program_display,
    program_from_command_line, program_id_for, program_notes,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::agui::AgUiWidget;

/// Model Context Protocol revision this client speaks first.
pub const PROTOCOL_MODERN: &str = "2026-07-28";
/// Previous Streamable HTTP revision, used only when a server rejects the modern one.
pub const PROTOCOL_LEGACY: &str = "2025-11-25";
/// Servers may answer with one JSON body or a server-sent event stream.
pub const ACCEPT_HEADER: &str = "application/json, text/event-stream";
/// Sent on every request so the server knows which revision this call speaks.
pub const PROTOCOL_HEADER: &str = "MCP-Protocol-Version";

const FORBIDDEN_STATUS_WORDS: &[&str] = &["JSON-RPC", "PKCE", "stdio", "Bearer", "OAuth"];

/// A web address Sovra is willing to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceAddress {
    /// Canonical form used for later requests and for binding a saved secret.
    pub url: String,
    pub host: String,
    pub loopback: bool,
}

/// Why an address was refused. The text is safe to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressProblem {
    Empty,
    NotAWebAddress,
    NotSecure,
    HasPassword,
}

impl AddressProblem {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Empty => "Paste the service's web address.",
            Self::NotAWebAddress | Self::HasPassword => {
                "That doesn't look like a web address. Copy it from the service and try again."
            }
            Self::NotSecure => {
                "Use a secure web address that starts with https. A service on this computer can use http://localhost."
            }
        }
    }
}

/// Accept `https` addresses. `http` is only allowed for this computer.
pub fn check_service_address(input: &str) -> Result<ServiceAddress, AddressProblem> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AddressProblem::Empty);
    }
    let url = Url::parse(trimmed).map_err(|_| AddressProblem::NotAWebAddress)?;
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(AddressProblem::HasPassword);
    }
    let host = url.host_str().ok_or(AddressProblem::NotAWebAddress)?;
    let loopback = is_loopback_host(host);
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => return Err(AddressProblem::NotSecure),
        _ => return Err(AddressProblem::NotAWebAddress),
    }
    Ok(ServiceAddress {
        url: url.to_string(),
        host: host.to_owned(),
        loopback,
    })
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// `true` when an access token saved for `token_server` may be attached to `request_url`.
pub fn token_may_be_sent(token_server: &str, request_url: &str) -> bool {
    !token_server.is_empty() && token_server == request_url
}

/// The `resource_metadata` URL from a `WWW-Authenticate` challenge, when the server sent one.
pub fn resource_metadata_parameter(header: &str) -> Option<String> {
    let needle = "resource_metadata=";
    let index = header.to_ascii_lowercase().find(needle)?;
    let rest = header.get(index + needle.len()..)?;
    let value = read_auth_param(rest);
    (!value.is_empty()).then_some(value)
}

fn read_auth_param(rest: &str) -> String {
    let bytes = rest.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut out = String::new();
        let mut escaped = false;
        for byte in rest[1..].chars() {
            if escaped {
                out.push(byte);
                escaped = false;
                continue;
            }
            match byte {
                '\\' => escaped = true,
                '"' => break,
                _ => out.push(byte),
            }
        }
        return out;
    }
    rest.split([',', ' ', '\t'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Well-known protected-resource URLs for a service that did not name one itself.
/// The path-specific URL comes first, then the host-wide one.
pub fn protected_resource_well_known_urls(resource: &str) -> Vec<String> {
    let Ok(url) = Url::parse(resource) else {
        return Vec::new();
    };
    let Some(origin) = origin_of(&url) else {
        return Vec::new();
    };
    let path = url.path().trim_matches('/');
    let mut urls = Vec::new();
    if !path.is_empty() {
        urls.push(format!(
            "{origin}/.well-known/oauth-protected-resource/{path}"
        ));
    }
    urls.push(format!("{origin}/.well-known/oauth-protected-resource"));
    urls
}

/// Authorization-server metadata URL for an issuer (RFC 8414 path insertion).
pub fn authorization_server_metadata_url(issuer: &str) -> Option<String> {
    let url = Url::parse(issuer).ok()?;
    let origin = origin_of(&url)?;
    let path = url.path().trim_matches('/');
    if path.is_empty() {
        Some(format!("{origin}/.well-known/oauth-authorization-server"))
    } else {
        Some(format!(
            "{origin}/.well-known/oauth-authorization-server/{path}"
        ))
    }
}

/// Origin header for a service address. IPv6 hosts stay in brackets.
pub fn request_origin(resource: &str) -> Option<String> {
    origin_of(&Url::parse(resource).ok()?)
}

fn origin_of(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let scheme = url.scheme();
    match url.port() {
        Some(port) => Some(format!("{scheme}://{host}:{port}")),
        None => Some(format!("{scheme}://{host}")),
    }
}

/// What a protected-resource document says about sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedResource {
    pub resource: String,
    pub authorization_servers: Vec<String>,
}

/// Reads a protected-resource document. An empty authorization-server list means
/// the service did not point at a sign-in page.
pub fn parse_protected_resource(body: &str) -> Option<ProtectedResource> {
    let value: Value = serde_json::from_str(body).ok()?;
    let resource = value.get("resource")?.as_str()?.to_owned();
    if resource.is_empty() {
        return None;
    }
    let authorization_servers = value
        .get("authorization_servers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(ProtectedResource {
        resource,
        authorization_servers,
    })
}

/// The sign-in endpoints a service pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationServer {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub supports_client_metadata: bool,
}

pub fn parse_authorization_server(body: &str) -> Option<AuthorizationServer> {
    let value: Value = serde_json::from_str(body).ok()?;
    let issuer = text_field(&value, "issuer")?;
    let authorization_endpoint = text_field(&value, "authorization_endpoint")?;
    let token_endpoint = text_field(&value, "token_endpoint")?;
    let registration_endpoint = text_field(&value, "registration_endpoint");
    let supports_client_metadata = value
        .get("client_id_metadata_document_supported")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(AuthorizationServer {
        issuer,
        authorization_endpoint,
        token_endpoint,
        registration_endpoint,
        supports_client_metadata,
    })
}

fn text_field(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// How Sovra will introduce itself to the sign-in service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationChoice {
    /// Details already saved for this exact sign-in service.
    Saved { client_id: String },
    /// The sign-in service accepts a hosted metadata document we already have.
    ClientMetadata { url: String },
    /// Ask the sign-in service to register this app.
    Dynamic { endpoint: String },
    /// The person has to paste the details the service gave them.
    AskPerson,
}

/// Inputs to [`choose_registration`], in the order the specification requires.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegistrationOffer<'a> {
    pub saved_client_id: Option<&'a str>,
    pub metadata_url: Option<&'a str>,
    pub server_supports_metadata: bool,
    pub registration_endpoint: Option<&'a str>,
}

pub fn choose_registration(offer: &RegistrationOffer<'_>) -> RegistrationChoice {
    if let Some(client_id) = offer
        .saved_client_id
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        return RegistrationChoice::Saved {
            client_id: client_id.to_owned(),
        };
    }
    if offer.server_supports_metadata
        && let Some(url) = offer.metadata_url.map(str::trim)
        && is_https_url(url)
    {
        return RegistrationChoice::ClientMetadata {
            url: url.to_owned(),
        };
    }
    if let Some(endpoint) = offer
        .registration_endpoint
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
        && Url::parse(endpoint).is_ok_and(|url| matches!(url.scheme(), "https" | "http"))
    {
        return RegistrationChoice::Dynamic {
            endpoint: endpoint.to_owned(),
        };
    }
    RegistrationChoice::AskPerson
}

fn is_https_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

/// Document sent when a sign-in service can register this app itself.
pub fn dynamic_registration_document(redirect_uri: &str) -> Value {
    json!({
        "client_name": "Sovra",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native"
    })
}

/// A present `iss` must be the issuer we asked. A missing `iss` is allowed.
pub fn authorization_issuer_ok(expected_issuer: &str, returned: Option<&str>) -> bool {
    match returned {
        None => true,
        Some(issuer) => !expected_issuer.is_empty() && issuer == expected_issuer,
    }
}

/// S256 challenge for a PKCE verifier (base64url, no padding).
pub fn s256_challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

pub fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = u32::from(bytes[index]) << 16
            | u32::from(bytes[index + 1]) << 8
            | u32::from(bytes[index + 2]);
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(chunk & 0x3f) as usize] as char);
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        let chunk = u32::from(bytes[index]) << 16;
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
    } else if rest == 2 {
        let chunk = u32::from(bytes[index]) << 16 | u32::from(bytes[index + 1]) << 8;
        out.push(TABLE[((chunk >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 0x3f) as usize] as char);
    }
    out
}

/// What the browser sent back to this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInCallback {
    pub code: String,
    pub state: String,
    pub issuer: Option<String>,
}

/// Parses `GET /callback?code=…&state=…&iss=…`.
pub fn parse_sign_in_callback(request_line: &str) -> Option<SignInCallback> {
    let target = request_line.strip_prefix("GET ")?.split(' ').next()?;
    let query = target.strip_prefix("/callback?")?;
    let mut code = None;
    let mut state = None;
    let mut issuer = None;
    let mut error = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decoded = crate::auth::percent_decode(value);
        match key {
            "code" => code = Some(decoded),
            "state" => state = Some(decoded),
            "iss" => issuer = Some(decoded),
            "error" => error = Some(decoded),
            _ => {}
        }
    }
    if error.is_some() {
        return None;
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            Some(SignInCallback {
                code,
                state,
                issuer: issuer.filter(|value| !value.is_empty()),
            })
        }
        _ => None,
    }
}

/// Which saved secret this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    /// Access token for one sign-in service and one connected service.
    Access,
    /// Registration details for one sign-in service.
    Client,
    /// A secret the service itself gave the person, with no sign-in page.
    Static,
    /// Hosted metadata URL already configured for one sign-in service.
    Metadata,
    /// Private settings for one service: a program's environment values or a web
    /// address's extra headers, as a JSON object.
    Settings,
}

/// A secret plus the only place it may be used. The value is omitted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRecord {
    pub issuer: String,
    pub server: String,
    pub kind: SecretKind,
    value: String,
}

impl std::fmt::Debug for SecretRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretRecord")
            .field("issuer", &self.issuer)
            .field("server", &self.server)
            .field("kind", &self.kind)
            .field("value", &"…")
            .finish()
    }
}

impl SecretRecord {
    pub fn new(
        issuer: impl Into<String>,
        server: impl Into<String>,
        kind: SecretKind,
        value: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            server: server.into(),
            kind,
            value: value.into(),
        }
    }

    pub fn reveal(&self) -> &str {
        &self.value
    }
}

/// Returns the secret only when it was saved for this issuer and this service.
pub fn accept_secret<'a>(
    record: &'a SecretRecord,
    issuer: &str,
    server: &str,
    kind: SecretKind,
) -> Option<&'a str> {
    if record.kind != kind {
        return None;
    }
    let matches = match kind {
        SecretKind::Access => record.issuer == issuer && record.server == server,
        SecretKind::Client | SecretKind::Metadata => record.issuer == issuer && !issuer.is_empty(),
        SecretKind::Static | SecretKind::Settings => {
            record.issuer.is_empty() && record.server == server && !server.is_empty()
        }
    };
    matches.then_some(record.reveal())
}

/// Storage id for one secret. Different issuers and services never share an id.
pub fn secret_storage_id(issuer: &str, server: &str, kind: SecretKind) -> String {
    let mut hasher = Sha256::new();
    hasher.update(issuer.as_bytes());
    hasher.update([0xff]);
    hasher.update(server.as_bytes());
    hasher.update([0xff]);
    hasher.update(match kind {
        SecretKind::Access => b"access".as_slice(),
        SecretKind::Client => b"client".as_slice(),
        SecretKind::Static => b"static".as_slice(),
        SecretKind::Metadata => b"metadata".as_slice(),
        SecretKind::Settings => b"settings".as_slice(),
    });
    let digest = hasher.finalize();
    format!("mcp-{}", hex_of(&digest[..16]))
}

pub fn service_id_for(address: &str) -> String {
    let digest = Sha256::digest(address.as_bytes());
    format!("svc-{}", hex_of(&digest[..6]))
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Tool name the chat model sees. The real service and tool names travel beside it.
pub fn tool_alias(service_id: &str, tool: &str) -> String {
    fn clean(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect()
    }
    bounded_tool_alias(format!("connected_{}_{}", clean(service_id), clean(tool)))
}

/// Words shown for a failed connect. None of them name the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    Address,
    Unreachable,
    SignIn,
    SignInMismatch,
    NeedsSecret,
    NeedsClientDetails,
    ToolUnavailable,
}

impl ConnectFailure {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Address => AddressProblem::NotSecure.text(),
            Self::Unreachable => {
                "Sovra couldn't reach that service. Check the web address and try again."
            }
            Self::SignIn => "This service needs you to sign in.",
            Self::SignInMismatch => {
                "Sign-in didn't finish because the sign-in page didn't match the service. Try again."
            }
            Self::NeedsSecret => {
                "This service needs the secret it gave you. Paste that secret and connect again."
            }
            Self::NeedsClientDetails => {
                "This service needs the sign-in name it gave you. Paste that name and continue."
            }
            Self::ToolUnavailable => "That service is no longer connected.",
        }
    }
}

/// Where a connection sits. The label is what the screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum McpPhase {
    #[default]
    Saved,
    Connecting,
    NeedsSignIn,
    NeedsClientDetails,
    Connected,
    Failed,
    /// A program waiting for the person to approve running it.
    NeedsApproval,
}

impl McpPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Saved => "Not connected",
            Self::Connecting => "Connecting…",
            Self::NeedsSignIn => "Sign in",
            Self::NeedsClientDetails => "Needs a detail from you",
            Self::Connected => "Connected",
            Self::Failed => "Needs attention",
            Self::NeedsApproval => "Waiting for your OK",
        }
    }
}

/// Every fixed sentence the connect screen can show.
pub fn user_facing_copy() -> Vec<&'static str> {
    vec![
        AddressProblem::Empty.text(),
        AddressProblem::NotAWebAddress.text(),
        AddressProblem::HasPassword.text(),
        AddressProblem::NotSecure.text(),
        ConnectFailure::Unreachable.text(),
        ConnectFailure::SignIn.text(),
        ConnectFailure::SignInMismatch.text(),
        ConnectFailure::NeedsSecret.text(),
        ConnectFailure::NeedsClientDetails.text(),
        ConnectFailure::ToolUnavailable.text(),
        McpPhase::Saved.label(),
        McpPhase::Connecting.label(),
        McpPhase::NeedsSignIn.label(),
        McpPhase::NeedsClientDetails.label(),
        McpPhase::Connected.label(),
        McpPhase::Failed.label(),
        McpPhase::NeedsApproval.label(),
        PAUSED_LABEL,
        "Removed.",
        "Connected.",
        TOOL_FAILED_TEXT,
    ]
}

/// Status shown for a service the person paused.
pub const PAUSED_LABEL: &str = "Paused";

/// Shown when a connected service could not finish an action.
pub const TOOL_FAILED_TEXT: &str = "The service couldn't do that. Try again.";

pub fn copy_is_plain(text: &str) -> bool {
    FORBIDDEN_STATUS_WORDS.iter().all(|word| {
        !text
            .to_ascii_lowercase()
            .contains(&word.to_ascii_lowercase())
    })
}

/// One thing a connected service can do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
}

/// A connected service as the screen shows it. Secrets never live here: header
/// and environment *names* are kept, their values are in the keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpService {
    pub id: String,
    pub name: String,
    /// The web address; for a program, its exact command (see [`Self::location`]).
    pub address: String,
    pub phase: McpPhase,
    pub message: String,
    pub tools: Vec<McpToolInfo>,
    #[serde(default)]
    pub transport: McpTransport,
    /// Paused services keep their settings but are not started or offered.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Tools the person switched off; the model never sees them.
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    /// Extra headers sent to a web address (names only).
    #[serde(default)]
    pub header_names: Vec<String>,
}

const fn default_enabled() -> bool {
    true
}

impl Default for McpService {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            address: String::new(),
            phase: McpPhase::default(),
            message: String::new(),
            tools: Vec::new(),
            transport: McpTransport::default(),
            enabled: true,
            disabled_tools: Vec::new(),
            header_names: Vec::new(),
        }
    }
}

impl McpService {
    /// The status word: "Paused" for a paused service, else the phase's label.
    pub fn label(&self) -> &'static str {
        if self.enabled {
            self.phase.label()
        } else {
            PAUSED_LABEL
        }
    }
}

/// Everything the connect screen renders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpState {
    pub services: Vec<McpService>,
    #[serde(default)]
    pub notice: Option<String>,
}

impl McpState {
    pub fn apply(&mut self, event: &McpEvent) {
        match event {
            McpEvent::Loaded(state) => *self = state.clone(),
            McpEvent::Upsert(service) => {
                if let Some(existing) = self.services.iter_mut().find(|item| item.id == service.id)
                {
                    *existing = service.clone();
                } else {
                    self.services.push(service.clone());
                }
                // Status changes keep the last notice (e.g. an import summary);
                // the screen clears it when the person starts something new.
            }
            McpEvent::Removed { id } => {
                self.services.retain(|service| service.id != *id);
                self.notice = Some("Removed.".to_owned());
            }
            McpEvent::Notice(notice) => self.notice = Some(notice.clone()),
            McpEvent::ToolResult { .. } | McpEvent::OpenSignIn { .. } => {}
        }
    }

    pub fn service(&self, id: &str) -> Option<&McpService> {
        self.services.iter().find(|service| service.id == id)
    }
}

/// What the screen asks the runtime to do. `Debug` hides secrets.
#[derive(Clone, PartialEq, Eq)]
pub enum McpCommand {
    Load,
    Connect {
        name: String,
        address: String,
        secret: Option<String>,
    },
    /// Add a program on this computer. It waits for [`McpCommand::Approve`] before
    /// it first runs.
    AddProgram {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        working_dir: Option<String>,
    },
    /// Add everything in pasted settings (see [`parse_server_settings`]).
    Import {
        text: String,
    },
    /// Add Sovra's own built-in sample MCP server, ready to use.
    AddSample,
    /// The person read the exact command and allows the program to run.
    Approve {
        id: String,
    },
    /// Pause (stop offering and stop the program) or resume a service.
    SetEnabled {
        id: String,
        enabled: bool,
    },
    /// Offer or hide one tool from the model.
    SetToolEnabled {
        id: String,
        tool: String,
        enabled: bool,
    },
    /// Connect again: restart a program or reconnect a web address.
    Reconnect {
        id: String,
    },
    Rename {
        id: String,
        name: String,
    },
    /// Continue sign-in after the person pastes the name the service gave them.
    SubmitClientDetails {
        id: String,
        client_id: String,
        client_secret: Option<String>,
    },
    /// Open sign-in again for a service that asked for it.
    SignIn {
        id: String,
    },
    Remove {
        id: String,
    },
    /// Call one tool on a connected service. `session_id` attributes the reply in chat.
    CallTool {
        service_id: String,
        tool: String,
        arguments: Value,
        session_id: Option<String>,
    },
}

impl std::fmt::Debug for McpCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load => formatter.write_str("Load"),
            Self::Connect {
                name,
                address,
                secret,
            } => formatter
                .debug_struct("Connect")
                .field("name", name)
                .field("address", address)
                .field("secret", &secret.as_ref().map(|_| "…"))
                .finish(),
            Self::SubmitClientDetails {
                id,
                client_id,
                client_secret,
            } => formatter
                .debug_struct("SubmitClientDetails")
                .field("id", id)
                .field("client_id", client_id)
                .field("client_secret", &client_secret.as_ref().map(|_| "…"))
                .finish(),
            Self::AddProgram {
                name,
                command,
                args,
                env,
                working_dir,
            } => formatter
                .debug_struct("AddProgram")
                .field("name", name)
                .field("command", command)
                .field("args", args)
                .field(
                    "env",
                    &env.iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>(),
                )
                .field("working_dir", working_dir)
                .finish(),
            Self::AddSample => formatter.write_str("AddSample"),
            Self::Import { text } => formatter
                .debug_struct("Import")
                .field("bytes", &text.len())
                .finish(),
            Self::Approve { id } => formatter.debug_struct("Approve").field("id", id).finish(),
            Self::SetEnabled { id, enabled } => formatter
                .debug_struct("SetEnabled")
                .field("id", id)
                .field("enabled", enabled)
                .finish(),
            Self::SetToolEnabled { id, tool, enabled } => formatter
                .debug_struct("SetToolEnabled")
                .field("id", id)
                .field("tool", tool)
                .field("enabled", enabled)
                .finish(),
            Self::Reconnect { id } => formatter.debug_struct("Reconnect").field("id", id).finish(),
            Self::Rename { id, name } => formatter
                .debug_struct("Rename")
                .field("id", id)
                .field("name", name)
                .finish(),
            Self::SignIn { id } => formatter.debug_struct("SignIn").field("id", id).finish(),
            Self::Remove { id } => formatter.debug_struct("Remove").field("id", id).finish(),
            Self::CallTool {
                service_id,
                tool,
                arguments,
                session_id,
            } => formatter
                .debug_struct("CallTool")
                .field("service_id", service_id)
                .field("tool", tool)
                .field("arguments", arguments)
                .field("session_id", session_id)
                .finish(),
        }
    }
}

/// What the runtime tells the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpEvent {
    Loaded(McpState),
    Upsert(McpService),
    Removed {
        id: String,
    },
    Notice(String),
    /// Open this page in the browser so the person can sign in.
    OpenSignIn {
        service_id: String,
        url: String,
    },
    /// A real tool result. `text` is what the service returned; `widget` is a
    /// native AG-UI card when the service sent one as structured content.
    ToolResult {
        service_id: String,
        service_name: String,
        tool: String,
        text: String,
        widget: Option<AgUiWidget>,
        session_id: Option<String>,
    },
}

/// Folds a tool result into the open chat so the reply names the service.
pub fn record_tool_result(state: &mut crate::WorkspaceState, event: &McpEvent) {
    let McpEvent::ToolResult {
        service_name,
        text,
        widget,
        session_id,
        ..
    } = event
    else {
        return;
    };
    let Some(session_id) = session_id else {
        return;
    };
    if state.session_id != *session_id {
        return;
    }
    state.push_service_reply(service_name, text, widget.clone());
}

/// Outcome of the first hello to a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitializeOutcome {
    /// Speak the modern revision. Do not require a session id.
    /// `tools` is `Some` when this reply was already the tool catalogue.
    Modern {
        name: Option<String>,
        tools: Option<Vec<McpToolInfo>>,
    },
    /// The server only speaks the previous revision and issued a session.
    Legacy {
        name: Option<String>,
    },
    /// The body was not a recognized modern error. Try the previous revision once.
    FallBack,
    /// The server speaks the modern revision. Correct the request instead of
    /// starting the older handshake.
    Retry,
    Failed,
}

/// Header and body disagreed, or a required header was missing.
const HEADER_MISMATCH: i64 = -32_020;
/// The server requires a capability this request did not offer.
const MISSING_CAPABILITY: i64 = -32_021;
/// The server listed the revisions it does speak.
const UNSUPPORTED_VERSION: i64 = -32_022;
/// A modern server uses this, with HTTP 404, when it does not implement the method.
const METHOD_NOT_FOUND: i64 = -32_601;

pub fn interpret_initialize(requested: &str, http_status: u16, body: &str) -> InitializeOutcome {
    if !(200..300).contains(&http_status) {
        return classify_rejection(requested, http_status, body);
    }
    let Some(value) = json_from_body(body) else {
        return if requested == PROTOCOL_MODERN {
            InitializeOutcome::FallBack
        } else {
            InitializeOutcome::Failed
        };
    };
    if value.get("error").is_some() {
        return classify_error_value(requested, http_status, &value);
    }
    let result = value.get("result").unwrap_or(&value);
    let name = server_name(result);
    if result.get("tools").is_some() {
        return InitializeOutcome::Modern {
            name,
            tools: Some(parse_tool_list(result)),
        };
    }
    let version = result
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(requested);
    match version {
        PROTOCOL_MODERN => InitializeOutcome::Modern { name, tools: None },
        PROTOCOL_LEGACY => InitializeOutcome::Legacy { name },
        _ if requested == PROTOCOL_MODERN => InitializeOutcome::FallBack,
        _ => InitializeOutcome::Failed,
    }
}

fn classify_rejection(requested: &str, status: u16, body: &str) -> InitializeOutcome {
    if requested != PROTOCOL_MODERN {
        return InitializeOutcome::Failed;
    }
    if let Some(value) = json_from_body(body) {
        return classify_error_value(requested, status, &value);
    }
    if matches!(status, 400 | 404 | 405) {
        InitializeOutcome::FallBack
    } else {
        InitializeOutcome::Failed
    }
}

fn classify_error_value(requested: &str, status: u16, value: &Value) -> InitializeOutcome {
    let Some(code) = value.pointer("/error/code").and_then(Value::as_i64) else {
        return if requested == PROTOCOL_MODERN && matches!(status, 400 | 404 | 405) {
            InitializeOutcome::FallBack
        } else {
            InitializeOutcome::Failed
        };
    };
    match code {
        HEADER_MISMATCH | MISSING_CAPABILITY => InitializeOutcome::Retry,
        UNSUPPORTED_VERSION => advertised_version(value),
        METHOD_NOT_FOUND => InitializeOutcome::Failed,
        _ if requested == PROTOCOL_MODERN && matches!(status, 400 | 404 | 405) => {
            InitializeOutcome::FallBack
        }
        _ => InitializeOutcome::Failed,
    }
}

fn advertised_version(value: &Value) -> InitializeOutcome {
    let supported = value
        .pointer("/error/data/supported")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if supported.contains(&PROTOCOL_MODERN) {
        InitializeOutcome::Retry
    } else if supported.contains(&PROTOCOL_LEGACY) {
        InitializeOutcome::FallBack
    } else {
        InitializeOutcome::Failed
    }
}

fn server_name(result: &Value) -> Option<String> {
    named(
        result
            .get("_meta")
            .and_then(|meta| meta.get("io.modelcontextprotocol/serverInfo")),
    )
    .or_else(|| named(result.get("serverInfo")))
}

fn named(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = value
        .as_str()
        .or_else(|| value.get("name").and_then(Value::as_str))?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

pub fn json_from_body(body: &str) -> Option<Value> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| json_from_sse(trimmed))
}

pub fn json_from_sse(body: &str) -> Option<Value> {
    let mut data = String::new();
    let mut parsed = None;
    for line in body.lines() {
        if line.is_empty() {
            if let Some(value) = finish_sse_event(&data) {
                parsed = Some(value);
            }
            data.clear();
            continue;
        }
        if let Some(payload) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(payload.trim_start());
        }
    }
    if let Some(value) = finish_sse_event(&data) {
        parsed = Some(value);
    }
    parsed
}

fn finish_sse_event(data: &str) -> Option<Value> {
    let data = data.trim();
    if data.is_empty() {
        return None;
    }
    serde_json::from_str(data).ok()
}

pub fn initialize_request(id: u64, version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": { "name": "Sovra", "version": "0.1.0" }
        }
    })
}

pub fn initialized_notification() -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
}

/// Mirrors the JSON-RPC method so a gateway can route without reading the body.
pub const METHOD_HEADER: &str = "Mcp-Method";
/// Mirrors `params.name` on `tools/call`.
pub const NAME_HEADER: &str = "Mcp-Name";

pub fn tools_list_request(id: u64, version: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": params_with_meta(json!({}), version)
    })
}

pub fn tools_call_request(id: u64, version: &str, name: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": params_with_meta(json!({ "name": name, "arguments": arguments }), version)
    })
}

fn params_with_meta(mut params: Value, version: &str) -> Value {
    if version == PROTOCOL_MODERN
        && let Value::Object(map) = &mut params
    {
        map.insert("_meta".to_owned(), protocol_meta(version));
    }
    params
}

fn protocol_meta(version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientInfo": { "name": "Sovra", "version": "0.1.0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// The tool or resource name a modern `tools/call` must also send as `Mcp-Name`.
pub fn mirrored_name(body: &Value) -> Option<&str> {
    match body.get("method").and_then(Value::as_str) {
        Some("tools/call" | "prompts/get") => body.pointer("/params/name").and_then(Value::as_str),
        Some("resources/read") => body.pointer("/params/uri").and_then(Value::as_str),
        _ => None,
    }
}

/// Header field value, using the spec's Base64 sentinel when the text is not plain ASCII.
pub fn header_field_value(value: &str) -> String {
    if header_safe(value) {
        value.to_owned()
    } else {
        format!("=?base64?{}?=", base64_std(value.as_bytes()))
    }
}

fn header_safe(value: &str) -> bool {
    !value.is_empty()
        && value == value.trim()
        && !(value.starts_with("=?base64?") && value.ends_with("?="))
        && value
            .bytes()
            .all(|byte| matches!(byte, 0x09 | 0x20 | 0x21..=0x7e))
}

fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = u32::from(bytes[index]) << 16
            | u32::from(bytes[index + 1]) << 8
            | u32::from(bytes[index + 2]);
        push_base64(&mut out, TABLE, chunk, 4);
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        push_base64(&mut out, TABLE, u32::from(bytes[index]) << 16, 2);
        out.push_str("==");
    } else if rest == 2 {
        let chunk = u32::from(bytes[index]) << 16 | u32::from(bytes[index + 1]) << 8;
        push_base64(&mut out, TABLE, chunk, 3);
        out.push('=');
    }
    out
}

fn push_base64(out: &mut String, table: &[u8], chunk: u32, count: usize) {
    let shifts = [18, 12, 6, 0];
    for shift in shifts.into_iter().take(count) {
        out.push(char::from(table[((chunk >> shift) & 0x3f) as usize]));
    }
}

pub fn rpc_result(value: &Value) -> Option<&Value> {
    if value.get("error").is_some() {
        return None;
    }
    value.get("result")
}

pub fn parse_tool_list(result: &Value) -> Vec<McpToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let description = tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .unwrap_or("Does one job for this service.")
                        .to_owned();
                    let input_schema = tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                    Some(McpToolInfo {
                        name: name.to_owned(),
                        description,
                        input_schema,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Text the service returned from a tool. Empty when the body has none.
pub fn parse_tool_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Native AG-UI surface a service returned as MCP `structuredContent`, if any.
///
/// The sample dice tool uses this so the roll is a card, not only chat text.
/// Other servers may send the same `protocol: "ag_ui"` envelope.
pub fn parse_tool_widget(result: &Value) -> Option<AgUiWidget> {
    let structured = result.get("structuredContent")?;
    if structured.get("protocol").and_then(Value::as_str) != Some("ag_ui") {
        return None;
    }
    serde_json::from_value(structured.get("widget")?.clone()).ok()
}

/// A connected tool offered to one chat turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOffer {
    pub service_id: String,
    pub service_name: String,
    pub tool: McpToolInfo,
}

/// What a chat turn asks the connected service to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCall {
    pub service_id: String,
    pub tool: String,
    pub arguments: Value,
    pub session_id: String,
}

/// The service's own text, plus the name to show beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteOutput {
    pub service_name: String,
    pub text: String,
    /// Present when the service returned a native AG-UI card.
    pub widget: Option<AgUiWidget>,
}

/// The runtime implements this. The chat adapter calls it; it does not invent results.
#[async_trait::async_trait]
pub trait RemoteToolCaller: Send + Sync + std::fmt::Debug {
    async fn call(&self, request: RemoteCall) -> Result<RemoteOutput, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agui::UiComponent;

    #[test]
    fn remote_https_is_accepted_and_other_remote_schemes_are_rejected() {
        let cases = [
            ("https://notes.example.com/mcp", true),
            ("https://notes.example.com", true),
            ("http://notes.example.com/mcp", false),
            ("http://127.0.0.2/mcp", false),
            ("http://localhost:9/mcp", true),
            ("http://127.0.0.1:9/mcp", true),
            ("http://[::1]:9/mcp", true),
            ("http://LOCALHOST/mcp", true),
            ("ftp://localhost/mcp", false),
            ("https://user:secret@notes.example.com/mcp", false),
            ("", false),
            ("not a url", false),
        ];
        for (input, allowed) in cases {
            assert_eq!(check_service_address(input).is_ok(), allowed, "{input}");
        }
        let address = check_service_address("http://127.0.0.1:9/mcp").expect("loopback");
        assert!(address.loopback);
        assert_eq!(address.host, "127.0.0.1");
    }

    #[test]
    fn resource_metadata_and_well_known_urls_follow_the_challenge() {
        let header = r#"Bearer realm="mcp", resource_metadata="https://notes.example.com/.well-known/oauth-protected-resource/mcp""#;
        assert_eq!(
            resource_metadata_parameter(header).as_deref(),
            Some("https://notes.example.com/.well-known/oauth-protected-resource/mcp")
        );
        assert_eq!(resource_metadata_parameter("Bearer realm=\"mcp\""), None);
        assert_eq!(
            protected_resource_well_known_urls("https://notes.example.com/mcp"),
            vec![
                "https://notes.example.com/.well-known/oauth-protected-resource/mcp".to_owned(),
                "https://notes.example.com/.well-known/oauth-protected-resource".to_owned(),
            ]
        );
        assert_eq!(
            authorization_server_metadata_url("http://127.0.0.1:9/as").as_deref(),
            Some("http://127.0.0.1:9/.well-known/oauth-authorization-server/as")
        );
    }

    #[test]
    fn protected_resource_without_a_sign_in_service_is_explicit() {
        let parsed = parse_protected_resource(
            r#"{"resource":"http://127.0.0.1:9/mcp","authorization_servers":[]}"#,
        )
        .expect("document");
        assert_eq!(parsed.authorization_servers, Vec::<String>::new());
        let server = parse_authorization_server(
            r#"{"issuer":"http://127.0.0.1:9/as","authorization_endpoint":"http://127.0.0.1:9/as/authorize","token_endpoint":"http://127.0.0.1:9/as/token","registration_endpoint":"http://127.0.0.1:9/as/register","client_id_metadata_document_supported":false}"#,
        )
        .expect("server");
        assert_eq!(server.issuer, "http://127.0.0.1:9/as");
        assert!(!server.supports_client_metadata);
    }

    #[test]
    fn registration_choice_follows_spec_order_and_refuses_insecure_metadata() {
        let dynamic = choose_registration(&RegistrationOffer {
            registration_endpoint: Some("http://127.0.0.1:9/as/register"),
            ..RegistrationOffer::default()
        });
        assert!(matches!(dynamic, RegistrationChoice::Dynamic { .. }));

        let metadata = choose_registration(&RegistrationOffer {
            metadata_url: Some("https://client.example/sovra.json"),
            server_supports_metadata: true,
            registration_endpoint: Some("https://auth.example/register"),
            ..RegistrationOffer::default()
        });
        assert_eq!(
            metadata,
            RegistrationChoice::ClientMetadata {
                url: "https://client.example/sovra.json".into()
            }
        );

        let insecure_metadata = choose_registration(&RegistrationOffer {
            metadata_url: Some("http://client.example/sovra.json"),
            server_supports_metadata: true,
            ..RegistrationOffer::default()
        });
        assert_eq!(insecure_metadata, RegistrationChoice::AskPerson);

        let saved = choose_registration(&RegistrationOffer {
            saved_client_id: Some("saved-app"),
            metadata_url: Some("https://client.example/sovra.json"),
            server_supports_metadata: true,
            registration_endpoint: Some("https://auth.example/register"),
        });
        assert_eq!(
            saved,
            RegistrationChoice::Saved {
                client_id: "saved-app".into()
            }
        );

        let document = dynamic_registration_document("http://127.0.0.1:40/callback");
        assert_eq!(document["application_type"], "native");
        assert_eq!(document["redirect_uris"][0], "http://127.0.0.1:40/callback");
    }

    #[test]
    fn mismatched_issuer_is_rejected_and_a_missing_issuer_is_allowed() {
        assert!(authorization_issuer_ok("https://auth.example", None));
        assert!(authorization_issuer_ok(
            "https://auth.example",
            Some("https://auth.example")
        ));
        assert!(!authorization_issuer_ok(
            "https://auth.example",
            Some("https://attacker.example")
        ));
        assert!(!authorization_issuer_ok(
            "https://auth.example",
            Some("https://auth.example/")
        ));
    }

    #[test]
    fn pkce_s256_matches_the_published_vector() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            s256_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn secrets_for_one_issuer_are_not_selected_for_another() {
        let access = SecretRecord::new(
            "https://issuer-a.example",
            "https://notes.example/mcp",
            SecretKind::Access,
            "token-a",
        );
        assert_eq!(
            accept_secret(
                &access,
                "https://issuer-a.example",
                "https://notes.example/mcp",
                SecretKind::Access
            ),
            Some("token-a")
        );
        assert_eq!(
            accept_secret(
                &access,
                "https://issuer-b.example",
                "https://notes.example/mcp",
                SecretKind::Access
            ),
            None
        );
        assert_eq!(
            accept_secret(
                &access,
                "https://issuer-a.example",
                "https://other.example/mcp",
                SecretKind::Access
            ),
            None
        );
        let client = SecretRecord::new("https://issuer-a.example", "", SecretKind::Client, "cid");
        assert_eq!(
            accept_secret(&client, "https://issuer-b.example", "", SecretKind::Client),
            None
        );
        assert_ne!(
            secret_storage_id(
                "https://issuer-a.example",
                "https://notes.example/mcp",
                SecretKind::Access
            ),
            secret_storage_id(
                "https://issuer-b.example",
                "https://notes.example/mcp",
                SecretKind::Access
            )
        );
        assert!(!token_may_be_sent(
            "https://notes.example/mcp",
            "https://other.example/mcp"
        ));
        assert!(token_may_be_sent(
            "https://notes.example/mcp",
            "https://notes.example/mcp"
        ));
        let rendered = format!("{access:?}");
        assert!(!rendered.contains("token-a"));
    }

    #[test]
    fn status_copy_avoids_protocol_jargon() {
        for text in user_facing_copy() {
            assert!(copy_is_plain(text), "{text}");
        }
    }

    #[test]
    fn initialize_falls_back_from_a_legacy_rejection_and_parses_sse() {
        let rejected = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"Unsupported protocol version"}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 400, rejected),
            InitializeOutcome::FallBack
        );
        let mismatch = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32020,"message":"Header mismatch: missing Mcp-Method header"}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 400, mismatch),
            InitializeOutcome::Retry
        );
        let advertised = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2025-11-25"],"requested":"2026-07-28"}}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 400, advertised),
            InitializeOutcome::FallBack
        );
        let still_modern = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28"],"requested":"2026-07-28"}}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 400, still_modern),
            InitializeOutcome::Retry
        );
        assert_eq!(
            interpret_initialize(
                PROTOCOL_MODERN,
                404,
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"Method not found"}}"#
            ),
            InitializeOutcome::Failed
        );
        let legacy = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","serverInfo":{"name":"Old notes"}}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_LEGACY, 200, legacy),
            InitializeOutcome::Legacy {
                name: Some("Old notes".into())
            }
        );
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2026-07-28\",\"serverInfo\":{\"name\":\"Notes\"}}}\n\n";
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 200, sse),
            InitializeOutcome::Modern {
                name: Some("Notes".into()),
                tools: None
            }
        );
        let listed = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"echo","description":"Repeat a short note."}],"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"Recipe box"}}}}"#;
        assert_eq!(
            interpret_initialize(PROTOCOL_MODERN, 200, listed),
            InitializeOutcome::Modern {
                name: Some("Recipe box".into()),
                tools: Some(vec![McpToolInfo {
                    name: "echo".into(),
                    description: "Repeat a short note.".into(),
                    input_schema: json!({"type": "object", "properties": {}}),
                }])
            }
        );
        let list = tools_list_request(3, PROTOCOL_MODERN);
        assert_eq!(list["method"], "tools/list");
        assert_eq!(
            list["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            PROTOCOL_MODERN
        );
        assert!(list["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object());
        let legacy_list = tools_list_request(4, PROTOCOL_LEGACY);
        assert!(legacy_list["params"].get("_meta").is_none());
        let call = tools_call_request(7, PROTOCOL_MODERN, "echo", &json!({"text": "hi"}));
        assert_eq!(
            call["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            PROTOCOL_MODERN
        );
        assert_eq!(mirrored_name(&call), Some("echo"));
        assert_eq!(header_field_value("echo"), "echo");
        assert_eq!(
            header_field_value("Hello, 世界"),
            "=?base64?SGVsbG8sIOS4lueVjA==?="
        );
        let listed = parse_tool_list(
            &json!({"tools":[{"name":"echo","description":"Repeats a short note."}]}),
        );
        assert_eq!(listed[0].name, "echo");
        assert_eq!(
            parse_tool_text(&json!({"content":[{"type":"text","text":"hello"}]})),
            "hello"
        );
    }

    #[test]
    fn sign_in_callback_keeps_the_issuer_when_the_server_sends_one() {
        let callback = parse_sign_in_callback(
            "GET /callback?code=abc&state=xyz&iss=https%3A%2F%2Fauth.example HTTP/1.1",
        )
        .expect("callback");
        assert_eq!(callback.code, "abc");
        assert_eq!(callback.state, "xyz");
        assert_eq!(callback.issuer.as_deref(), Some("https://auth.example"));
        assert!(parse_sign_in_callback("GET /favicon.ico HTTP/1.1").is_none());
    }

    #[test]
    fn reducer_adds_and_removes_without_storing_a_secret() {
        let mut state = McpState::default();
        let service = McpService {
            id: "svc-1".into(),
            name: "Notes".into(),
            address: "https://notes.example/mcp".into(),
            phase: McpPhase::Connected,
            message: McpPhase::Connected.label().into(),
            tools: vec![McpToolInfo {
                name: "echo".into(),
                description: "Repeats a short note.".into(),
                input_schema: json!({}),
            }],
            ..McpService::default()
        };
        state.apply(&McpEvent::Upsert(service.clone()));
        assert_eq!(state.services.len(), 1);
        assert_eq!(state.service("svc-1").expect("saved").label(), "Connected");
        state.apply(&McpEvent::Removed { id: "svc-1".into() });
        assert_eq!(state.services, Vec::<McpService>::new());
        assert_eq!(state.notice.as_deref(), Some("Removed."));
        let rendered = format!("{service:?}");
        assert!(!rendered.to_ascii_lowercase().contains("bearer"));
    }

    #[test]
    fn a_tool_result_is_a_separate_reply_that_names_the_service() {
        let mut workspace = crate::WorkspaceState::new(0);
        let session_id = workspace.session_id.clone();
        let followed = workspace.apply_event(&crate::Event::Mcp(McpEvent::ToolResult {
            service_id: "svc-1".into(),
            service_name: "Recipe box".into(),
            tool: "echo".into(),
            text: "open-service-said-hello".into(),
            widget: None,
            session_id: Some(session_id),
        }));
        assert_eq!(followed, None);
        let message = workspace.messages.last().expect("reply");
        assert_eq!(message.text, "open-service-said-hello");
        assert_eq!(message.author, "Recipe box");
        let speaker = message.speaker.as_ref().expect("speaker");
        assert_eq!(speaker.name, "Recipe box");
        assert_eq!(speaker.role, "Connected service");
        assert_eq!(message.widgets, Vec::<AgUiWidget>::new());
    }

    #[test]
    fn a_structured_ag_ui_result_is_a_native_card_on_the_service_reply() {
        let widget = AgUiWidget::Components {
            surface_id: "sample-dice".into(),
            version: 1,
            components: vec![UiComponent::Card {
                title: "Dice roll".into(),
                description: "6-sided dice".into(),
                children: vec![UiComponent::Text {
                    text: "🎲 You rolled 1".into(),
                    heading: true,
                }],
            }],
        };
        let encoded = json!({
            "content": [{"type": "text", "text": "🎲 You rolled 1 (on a 6-sided dice)."}],
            "structuredContent": {
                "protocol": "ag_ui",
                "widget": widget,
            }
        });
        assert_eq!(
            parse_tool_text(&encoded),
            "🎲 You rolled 1 (on a 6-sided dice)."
        );
        assert_eq!(parse_tool_widget(&encoded), Some(widget.clone()));
        assert!(parse_tool_widget(&json!({"structuredContent": {"roll": 1}})).is_none());

        let mut workspace = crate::WorkspaceState::new(0);
        let session_id = workspace.session_id.clone();
        workspace.apply_event(&crate::Event::Mcp(McpEvent::ToolResult {
            service_id: "svc-sample".into(),
            service_name: "Sovra sample tools".into(),
            tool: "roll_dice".into(),
            text: "🎲 You rolled 1 (on a 6-sided dice).".into(),
            widget: Some(widget.clone()),
            session_id: Some(session_id),
        }));
        let message = workspace.messages.last().expect("reply");
        assert_eq!(message.author, "Sovra sample tools");
        assert_eq!(message.widgets, vec![widget]);
    }
}
