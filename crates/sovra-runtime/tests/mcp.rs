//! Loopback services for connect, sign-in, tool calls, and removal.
//!
//! This is the only test in the binary so a process-wide tracing subscriber can
//! capture the runtime log. The fixture checks the sign-in verifier itself.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sovra_core::mcp::{ConnectFailure, McpCommand, McpEvent, McpPhase};
use sovra_core::{Command, Event};
use sovra_runtime::Client;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

const ACCESS_TOKEN: &str = "fixture-access-token-9f3a";
const CLIENT_SECRET: &str = "fixture-client-secret-c4e1";
const LEGACY_SESSION: &str = "legacy-session-7b";
const OPEN_RESULT: &str = "open-service-said-hello";
const LEGACY_RESULT: &str = "legacy-service-said-hello";
const SIGNED_IN_RESULT: &str = "signed-in-service-said-hello";
const WAIT: Duration = Duration::from_secs(20);

#[derive(Clone, Copy)]
enum Kind {
    Open,
    Legacy,
    SignedIn,
}

struct Incoming {
    path: String,
    query: Vec<(String, String)>,
    headers: BTreeMap<String, String>,
    body: String,
}

struct World {
    kind: Kind,
    origin: String,
    mcp_url: String,
    issuer: String,
    hits: Vec<Incoming>,
    legacy_ready: bool,
    challenge: Option<String>,
    redirect: Option<String>,
    bearer_off_mcp: bool,
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        Self(Arc::clone(&self.0))
    }
}

fn log_buffer() -> Arc<Mutex<Vec<u8>>> {
    static BUFFER: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    Arc::clone(BUFFER.get_or_init(|| Arc::new(Mutex::new(Vec::new()))))
}

fn install_log() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(LogWriter(log_buffer()))
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("this binary has one test");
}

#[tokio::test]
async fn connects_calls_and_removes_three_loopback_services() {
    install_log();
    let client = sovra_runtime::start("mcp-test").expect("runtime");
    let events = client.events();
    let mut transcript = Vec::new();
    wait_ready(&events, &mut transcript).await;
    client.send(Command::Mcp(McpCommand::Load)).expect("load");
    match wait_mcp(&events, &mut transcript).await {
        McpEvent::Loaded(state) => assert_eq!(state.services.len(), 0),
        other => panic!("expected an empty list, got {other:?}"),
    }

    exercise(&client, &events, &mut transcript, Kind::Open, OPEN_RESULT).await;
    exercise(
        &client,
        &events,
        &mut transcript,
        Kind::Legacy,
        LEGACY_RESULT,
    )
    .await;
    exercise(
        &client,
        &events,
        &mut transcript,
        Kind::SignedIn,
        SIGNED_IN_RESULT,
    )
    .await;

    let logs = String::from_utf8_lossy(&log_buffer().lock().expect("log").clone()).into_owned();
    let spoken = transcript.join("\n");
    assert!(
        !logs.contains(ACCESS_TOKEN) && !spoken.contains(ACCESS_TOKEN),
        "access token leaked into the client log or an event"
    );
    assert!(
        !logs.contains(CLIENT_SECRET) && !spoken.contains(CLIENT_SECRET),
        "client secret leaked into the client log or an event"
    );
    client.send(Command::Shutdown).expect("shutdown");
}

async fn exercise(
    client: &Client,
    events: &async_channel::Receiver<Event>,
    transcript: &mut Vec<String>,
    kind: Kind,
    result: &str,
) {
    let (address, world) = bind(kind).await;
    client
        .send(Command::Mcp(McpCommand::Connect {
            name: String::new(),
            address: address.clone(),
            secret: None,
        }))
        .expect("connect");
    let service = wait_connected(events, transcript).await;
    assert_eq!(
        service.name,
        match kind {
            Kind::Open => "Open notes",
            Kind::Legacy => "Legacy notes",
            Kind::SignedIn => "Recipe box",
        }
    );
    assert_eq!(service.tools.len(), 1);
    assert_eq!(service.tools[0].name, "echo");
    assert_eq!(service.phase, McpPhase::Connected);
    let before_call = world.lock().expect("world").hits.len();
    client
        .send(Command::Mcp(McpCommand::CallTool {
            service_id: service.id.clone(),
            tool: "echo".to_owned(),
            arguments: json!({"text": "hi"}),
            session_id: Some("chat-1".to_owned()),
        }))
        .expect("call");
    match wait_mcp(events, transcript).await {
        McpEvent::ToolResult {
            service_name,
            text,
            session_id,
            tool,
            ..
        } => {
            assert_eq!(service_name, service.name);
            assert_eq!(text, result);
            assert_eq!(tool, "echo");
            assert_eq!(session_id.as_deref(), Some("chat-1"));
        }
        other => panic!("expected the service result, got {other:?}"),
    }
    assert!(world.lock().expect("world").hits.len() > before_call);
    let after_call = world.lock().expect("world").hits.len();
    client
        .send(Command::Mcp(McpCommand::Remove {
            id: service.id.clone(),
        }))
        .expect("remove");
    match wait_mcp(events, transcript).await {
        McpEvent::Removed { id } => assert_eq!(id, service.id),
        other => panic!("expected removal, got {other:?}"),
    }
    client
        .send(Command::Mcp(McpCommand::CallTool {
            service_id: service.id,
            tool: "echo".to_owned(),
            arguments: json!({}),
            session_id: Some("chat-1".to_owned()),
        }))
        .expect("call after removal");
    match wait_mcp(events, transcript).await {
        McpEvent::ToolResult { text, .. } => {
            assert_eq!(text, ConnectFailure::ToolUnavailable.text());
        }
        other => panic!("expected a stopped service, got {other:?}"),
    }
    assert_eq!(world.lock().expect("world").hits.len(), after_call);
    assert_service(&world.lock().expect("world"));
}

fn assert_service(world: &World) {
    match world.kind {
        Kind::Open => assert_open(world),
        Kind::Legacy => assert_legacy(world),
        Kind::SignedIn => assert_signed_in(world),
    }
}

fn assert_open(world: &World) {
    let methods = methods(world);
    assert_eq!(methods, vec!["tools/list", "tools/call"]);
    for hit in &world.hits {
        assert_eq!(hit.path, "/mcp");
        assert!(!hit.headers.contains_key("authorization"));
        assert!(!hit.headers.contains_key("mcp-session-id"));
        assert_mcp_headers(hit, "2026-07-28", &world.origin);
    }
    assert_modern_request(&world.hits[0], "tools/list");
    assert_modern_request(&world.hits[1], "tools/call");
}

fn assert_legacy(world: &World) {
    assert_eq!(
        methods(world),
        vec![
            "tools/list",
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call"
        ]
    );
    assert_modern_request(&world.hits[0], "tools/list");
    assert!(!world.hits[0].headers.contains_key("mcp-session-id"));
    for hit in world.hits.iter().skip(1) {
        assert_eq!(
            hit.headers.get("mcp-protocol-version").map(String::as_str),
            Some("2025-11-25")
        );
        assert_mcp_headers(hit, "2025-11-25", &world.origin);
        assert!(!hit.headers.contains_key("mcp-method"));
    }
    for hit in world.hits.iter().skip(2) {
        assert_eq!(
            hit.headers.get("mcp-session-id").map(String::as_str),
            Some(LEGACY_SESSION)
        );
    }
    assert!(
        world
            .hits
            .iter()
            .all(|hit| !hit.headers.contains_key("authorization"))
    );
}

fn assert_signed_in(world: &World) {
    assert!(!world.bearer_off_mcp, "bearer left the service address");
    let paths: Vec<&str> = world.hits.iter().map(|hit| hit.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/mcp",
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-authorization-server/as",
            "/register",
            "/authorize",
            "/token",
            "/mcp",
            "/mcp",
        ]
    );
    assert_modern_request(&world.hits[0], "tools/list");
    assert!(!world.hits[0].headers.contains_key("authorization"));
    for hit in world.hits.iter().take(6).skip(1) {
        assert!(!hit.headers.contains_key("authorization"));
    }
    for hit in world.hits.iter().skip(6) {
        let mut expected = String::from("Bearer ");
        expected.push_str(ACCESS_TOKEN);
        assert_eq!(
            hit.headers.get("authorization").map(String::as_str),
            Some(expected.as_str())
        );
        assert!(!hit.headers.contains_key("mcp-session-id"));
        assert_mcp_headers(hit, "2026-07-28", &world.origin);
    }
    assert_modern_request(&world.hits[6], "tools/list");
    assert_modern_request(&world.hits[7], "tools/call");
    let registration: Value = serde_json::from_str(&world.hits[3].body).expect("registration");
    assert_eq!(registration["application_type"], "native");
    assert_eq!(registration["token_endpoint_auth_method"], "none");
    assert_eq!(registration["client_name"], "Sovra");
    assert_eq!(registration["grant_types"][0], "authorization_code");
    let authorize = &world.hits[4];
    assert!(
        authorize
            .query
            .iter()
            .all(|(key, _)| key != "client_secret")
    );
    assert_eq!(
        query(authorize, "resource").as_deref(),
        Some(world.mcp_url.as_str())
    );
    assert_eq!(
        query(authorize, "code_challenge_method").as_deref(),
        Some("S256")
    );
    let token_body = form(&world.hits[5].body);
    assert_eq!(
        token_body.get("resource").map(String::as_str),
        Some(world.mcp_url.as_str())
    );
    let verifier = token_body
        .get("code_verifier")
        .map(String::as_str)
        .unwrap_or_default();
    assert_eq!(
        challenge_of(verifier).as_str(),
        world.challenge.as_deref().unwrap_or_default()
    );
}

fn methods(world: &World) -> Vec<&str> {
    world
        .hits
        .iter()
        .map(|hit| {
            serde_json::from_str::<Value>(&hit.body)
                .ok()
                .and_then(|value| {
                    value
                        .get("method")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default()
        })
        .map(|method| match method.as_str() {
            "initialize" => "initialize",
            "notifications/initialized" => "notifications/initialized",
            "tools/list" => "tools/list",
            "tools/call" => "tools/call",
            _ => "",
        })
        .collect()
}

fn assert_modern_request(hit: &Incoming, method: &str) {
    assert_eq!(
        hit.headers.get("mcp-method").map(String::as_str),
        Some(method)
    );
    let body: Value = serde_json::from_str(&hit.body).expect("request body");
    assert_eq!(body["method"], method);
    let meta_version = body
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str);
    assert_eq!(meta_version, Some("2026-07-28"));
    assert_eq!(
        hit.headers.get("mcp-protocol-version").map(String::as_str),
        Some("2026-07-28")
    );
    if method == "tools/call" {
        assert_eq!(
            hit.headers.get("mcp-name").map(String::as_str),
            Some("echo")
        );
        assert_eq!(body["params"]["name"], "echo");
    }
}

fn assert_mcp_headers(hit: &Incoming, protocol: &str, origin: &str) {
    let accept = hit
        .headers
        .get("accept")
        .map(String::as_str)
        .unwrap_or_default();
    assert!(accept.contains("application/json"), "{accept}");
    assert!(accept.contains("text/event-stream"), "{accept}");
    assert_eq!(
        hit.headers.get("mcp-protocol-version").map(String::as_str),
        Some(protocol)
    );
    assert_eq!(hit.headers.get("origin").map(String::as_str), Some(origin));
}

fn query(hit: &Incoming, name: &str) -> Option<String> {
    hit.query
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

fn form(body: &str) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// Independent S256 of the verifier. This does not call the client's helper.
fn challenge_of(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= bytes.len() {
        let chunk = u32::from(bytes[index]) << 16
            | u32::from(bytes[index + 1]) << 8
            | u32::from(bytes[index + 2]);
        out.push(char::from(TABLE[((chunk >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((chunk >> 12) & 0x3f) as usize]));
        out.push(char::from(TABLE[((chunk >> 6) & 0x3f) as usize]));
        out.push(char::from(TABLE[(chunk & 0x3f) as usize]));
        index += 3;
    }
    let rest = bytes.len() - index;
    if rest == 1 {
        let chunk = u32::from(bytes[index]) << 16;
        out.push(char::from(TABLE[((chunk >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((chunk >> 12) & 0x3f) as usize]));
    } else if rest == 2 {
        let chunk = u32::from(bytes[index]) << 16 | u32::from(bytes[index + 1]) << 8;
        out.push(char::from(TABLE[((chunk >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((chunk >> 12) & 0x3f) as usize]));
        out.push(char::from(TABLE[((chunk >> 6) & 0x3f) as usize]));
    }
    out
}

async fn bind(kind: Kind) -> (String, Arc<Mutex<World>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("port").port();
    let origin = format!("http://127.0.0.1:{port}");
    let mcp_url = format!("{origin}/mcp");
    let world = Arc::new(Mutex::new(World {
        kind,
        issuer: format!("{origin}/as"),
        origin,
        mcp_url: mcp_url.clone(),
        hits: Vec::new(),
        legacy_ready: false,
        challenge: None,
        redirect: None,
        bearer_off_mcp: false,
    }));
    let shared = Arc::clone(&world);
    tokio::runtime::Handle::current().spawn(async move {
        serve(listener, shared).await;
    });
    (mcp_url, world)
}

async fn serve(listener: TcpListener, world: Arc<Mutex<World>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            break;
        };
        let world = Arc::clone(&world);
        let request = tokio::time::timeout(Duration::from_secs(5), read_http(&mut stream)).await;
        let Ok(Some(request)) = request else {
            continue;
        };
        let response = {
            let mut world = world.lock().expect("world");
            dispatch(&mut world, &request)
        };
        let _ = stream.write_all(&response).await;
        let _ = stream.shutdown().await;
    }
}

fn dispatch(world: &mut World, request: &Incoming) -> Vec<u8> {
    if request.headers.contains_key("authorization") && request.path != "/mcp" {
        world.bearer_off_mcp = true;
    }
    let response = match world.kind {
        Kind::Open => dispatch_open(request, OPEN_RESULT, "Open notes", true),
        Kind::Legacy => dispatch_legacy(world, request),
        Kind::SignedIn => dispatch_signed_in(world, request),
    };
    world.hits.push(Incoming {
        path: request.path.clone(),
        query: request.query.clone(),
        headers: request.headers.clone(),
        body: request.body.clone(),
    });
    response
}

fn dispatch_open(request: &Incoming, result: &str, name: &str, list_as_sse: bool) -> Vec<u8> {
    if let Some(rejected) = reject_modern(request) {
        return rejected;
    }
    let method = method_of(request);
    let catalogue = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "_meta": {
                "io.modelcontextprotocol/serverInfo": { "name": name, "version": "1.0.0" }
            },
            "tools": [{
                "name": "echo",
                "description": "Repeat a short note.",
                "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}
            }]
        }
    });
    match method.as_deref() {
        Some("tools/list") if list_as_sse => sse(&catalogue),
        Some("tools/list") => json_ok(&catalogue),
        Some("tools/call") => json_ok(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {"content": [{"type": "text", "text": result}]}
        })),
        Some("initialize" | "notifications/initialized") => header_mismatch(),
        _ => json_status(
            404,
            &json!({"jsonrpc":"2.0","error":{"code":-32601,"message":"method"}}),
        ),
    }
}

fn dispatch_legacy(world: &mut World, request: &Incoming) -> Vec<u8> {
    if request.path != "/mcp" {
        return empty(404, "Not Found");
    }
    if request.headers.contains_key("authorization") || !accepts_both(request) {
        return json_status(
            400,
            &json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"headers"}}),
        );
    }
    if !request.headers.contains_key("origin") {
        return json_status(
            400,
            &json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"origin"}}),
        );
    }
    if request
        .headers
        .get("mcp-protocol-version")
        .map(String::as_str)
        == Some("2026-07-28")
    {
        return json_status(
            400,
            &json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"unsupported protocol version"}}),
        );
    }
    if request
        .headers
        .get("mcp-protocol-version")
        .map(String::as_str)
        != Some("2025-11-25")
    {
        return json_status(
            400,
            &json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"version"}}),
        );
    }
    match method_of(request).as_deref() {
        Some("initialize") => {
            world.legacy_ready = false;
            let mut response = json_ok(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "serverInfo": { "name": "Legacy notes" }
                }
            }));
            insert_header(&mut response, "Mcp-Session-Id", LEGACY_SESSION);
            response
        }
        Some("notifications/initialized") if session_ok(request) => {
            world.legacy_ready = true;
            empty(202, "Accepted")
        }
        Some("tools/list") if world.legacy_ready && session_ok(request) => json_ok(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {"tools": [{"name": "echo", "description": "Repeat a short note."}]}
        })),
        Some("tools/call") if world.legacy_ready && session_ok(request) => json_ok(&json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {"content": [{"type": "text", "text": LEGACY_RESULT}]}
        })),
        _ => json_status(
            400,
            &json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"session"}}),
        ),
    }
}

fn dispatch_signed_in(world: &mut World, request: &Incoming) -> Vec<u8> {
    match request.path.as_str() {
        "/mcp" => {
            if let Some(rejected) = reject_modern(request) {
                return rejected;
            }
            let mut expected = String::from("Bearer ");
            expected.push_str(ACCESS_TOKEN);
            if request.headers.get("authorization").map(String::as_str) != Some(expected.as_str()) {
                let mut challenge = String::from("Bearer realm=\"mcp\", resource_metadata=\"");
                challenge.push_str(&world.origin);
                challenge.push_str("/.well-known/oauth-protected-resource/mcp\"");
                return with_header(empty(401, "Unauthorized"), "WWW-Authenticate", &challenge);
            }
            dispatch_open(request, SIGNED_IN_RESULT, "Recipe box", false)
        }
        "/.well-known/oauth-protected-resource/mcp" => json_ok(&json!({
            "resource": world.mcp_url,
            "authorization_servers": [world.issuer]
        })),
        "/.well-known/oauth-authorization-server/as" => json_ok(&json!({
            "issuer": world.issuer,
            "authorization_endpoint": format!("{}/authorize", world.origin),
            "token_endpoint": format!("{}/token", world.origin),
            "registration_endpoint": format!("{}/register", world.origin),
            "client_id_metadata_document_supported": false
        })),
        "/register" => {
            let body: Value = serde_json::from_str(&request.body).unwrap_or(Value::Null);
            if body.get("application_type").and_then(Value::as_str) != Some("native") {
                return json_status(400, &json!({"error": "invalid_client_metadata"}));
            }
            json_status(
                201,
                &json!({
                    "client_id": "fixture-client",
                    "client_secret": CLIENT_SECRET
                }),
            )
        }
        "/authorize" => authorize(world, request),
        "/token" => issue_token(world, request),
        _ => empty(404, "Not Found"),
    }
}

fn authorize(world: &mut World, request: &Incoming) -> Vec<u8> {
    if request.query.iter().any(|(key, _)| key == "client_secret") {
        return empty(400, "Bad Request");
    }
    let Some(redirect) = query(request, "redirect_uri") else {
        return empty(400, "Bad Request");
    };
    let Some(state) = query(request, "state") else {
        return empty(400, "Bad Request");
    };
    let Some(challenge) = query(request, "code_challenge") else {
        return empty(400, "Bad Request");
    };
    if query(request, "resource").as_deref() != Some(world.mcp_url.as_str()) {
        return empty(400, "Bad Request");
    }
    world.challenge = Some(challenge);
    world.redirect = Some(redirect.clone());
    let mut location = url::Url::parse(&redirect)
        .unwrap_or_else(|_| url::Url::parse("http://127.0.0.1/callback").unwrap());
    {
        let mut pairs = location.query_pairs_mut();
        pairs.append_pair("code", "fixture-code");
        pairs.append_pair("state", &state);
        pairs.append_pair("iss", &world.issuer);
    }
    with_header(empty(302, "Found"), "Location", location.as_str())
}

fn issue_token(world: &World, request: &Incoming) -> Vec<u8> {
    let body = form(&request.body);
    let verifier = body
        .get("code_verifier")
        .map(String::as_str)
        .unwrap_or_default();
    let matches = world.challenge.as_deref() == Some(challenge_of(verifier).as_str())
        && body.get("resource").map(String::as_str) == Some(world.mcp_url.as_str())
        && body.get("redirect_uri").map(String::as_str) == world.redirect.as_deref()
        && body.get("code").map(String::as_str) == Some("fixture-code");
    if !matches {
        return json_status(400, &json!({"error": "invalid_grant"}));
    }
    json_ok(&json!({
        "access_token": ACCESS_TOKEN,
        "token_type": "Bearer",
        "iss": world.issuer
    }))
}

fn reject_modern(request: &Incoming) -> Option<Vec<u8>> {
    if request.path != "/mcp"
        || request.headers.contains_key("mcp-session-id")
        || !accepts_both(request)
        || request
            .headers
            .get("mcp-protocol-version")
            .map(String::as_str)
            != Some("2026-07-28")
        || request.headers.get("origin").map(String::as_str).is_none()
    {
        return Some(json_status(
            400,
            &json!({"jsonrpc":"2.0","error":{"code":-32600,"message":"headers"}}),
        ));
    }
    let body = serde_json::from_str::<Value>(&request.body).unwrap_or(Value::Null);
    let method = body.get("method").and_then(Value::as_str);
    if method.is_none() || request.headers.get("mcp-method").map(String::as_str) != method {
        return Some(header_mismatch());
    }
    let meta_version = body
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str);
    if meta_version != Some("2026-07-28") {
        return Some(header_mismatch());
    }
    if method == Some("tools/call") {
        let name = body.pointer("/params/name").and_then(Value::as_str);
        if name.is_none() || request.headers.get("mcp-name").map(String::as_str) != name {
            return Some(header_mismatch());
        }
    }
    None
}

fn header_mismatch() -> Vec<u8> {
    json_status(
        400,
        &json!({"jsonrpc":"2.0","error":{"code":-32020,"message":"Header mismatch"}}),
    )
}

fn accepts_both(request: &Incoming) -> bool {
    let accept = request
        .headers
        .get("accept")
        .map(String::as_str)
        .unwrap_or_default();
    accept.contains("application/json") && accept.contains("text/event-stream")
}

fn session_ok(request: &Incoming) -> bool {
    request.headers.get("mcp-session-id").map(String::as_str) == Some(LEGACY_SESSION)
}

fn method_of(request: &Incoming) -> Option<String> {
    serde_json::from_str::<Value>(&request.body)
        .ok()?
        .get("method")?
        .as_str()
        .map(str::to_owned)
}

async fn read_http(stream: &mut TcpStream) -> Option<Incoming> {
    let mut buf = Vec::new();
    let mut temp = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut temp).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&temp[..read]);
        if let Some(end) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if buf.len() > 65_536 {
            return None;
        }
    };
    let header_text = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let _method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < length {
        let read = stream.read(&mut temp).await.ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&temp[..read]);
    }
    body.truncate(length);
    let parsed = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    Some(Incoming {
        path: parsed.path().to_owned(),
        query: parsed
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn json_ok(body: &Value) -> Vec<u8> {
    json_status(200, body)
}

fn json_status(status: u16, body: &Value) -> Vec<u8> {
    let reason = match status {
        201 => "Created",
        400 => "Bad Request",
        _ => "OK",
    };
    with_header(
        bytes(status, reason, &body.to_string()),
        "Content-Type",
        "application/json",
    )
}

fn sse(body: &Value) -> Vec<u8> {
    let mut payload = String::from("data: ");
    payload.push_str(&body.to_string());
    payload.push_str("\n\n");
    with_header(
        bytes(200, "OK", &payload),
        "Content-Type",
        "text/event-stream",
    )
}

fn empty(status: u16, reason: &str) -> Vec<u8> {
    bytes(status, reason, "")
}

fn bytes(status: u16, reason: &str, body: &str) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    head.push_str(body);
    head.into_bytes()
}

fn with_header(mut response: Vec<u8>, name: &str, value: &str) -> Vec<u8> {
    let head = String::from_utf8_lossy(&response).into_owned();
    let Some((prefix, rest)) = head.split_once("\r\n") else {
        return response;
    };
    let mut rebuilt = prefix.to_owned();
    rebuilt.push_str("\r\n");
    rebuilt.push_str(name);
    rebuilt.push_str(": ");
    rebuilt.push_str(value);
    rebuilt.push_str("\r\n");
    rebuilt.push_str(rest);
    response = rebuilt.into_bytes();
    response
}

fn insert_header(response: &mut Vec<u8>, name: &str, value: &str) {
    *response = with_header(std::mem::take(response), name, value);
}

async fn wait_ready(events: &async_channel::Receiver<Event>, transcript: &mut Vec<String>) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = recv(events, deadline).await;
        record(transcript, &event);
        if matches!(event, Event::Ready { .. }) {
            return;
        }
    }
}

async fn wait_connected(
    events: &async_channel::Receiver<Event>,
    transcript: &mut Vec<String>,
) -> sovra_core::mcp::McpService {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = recv(events, deadline).await;
        record(transcript, &event);
        match event {
            Event::Mcp(McpEvent::Upsert(service)) if service.phase == McpPhase::Connected => {
                return service;
            }
            Event::Mcp(McpEvent::Upsert(service)) if service.phase == McpPhase::Failed => {
                panic!("connect failed: {}", service.message);
            }
            Event::Mcp(McpEvent::Notice(notice)) => panic!("connect refused: {notice}"),
            _ => {}
        }
    }
}

async fn wait_mcp(
    events: &async_channel::Receiver<Event>,
    transcript: &mut Vec<String>,
) -> McpEvent {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = recv(events, deadline).await;
        record(transcript, &event);
        if let Event::Mcp(event) = event {
            return event;
        }
    }
}

async fn recv(events: &async_channel::Receiver<Event>, deadline: tokio::time::Instant) -> Event {
    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
    tokio::time::timeout(remaining, events.recv())
        .await
        .expect("timed out waiting for the runtime")
        .expect("runtime stopped")
}

fn record(transcript: &mut Vec<String>, event: &Event) {
    transcript.push(format!("{event:?}"));
}
