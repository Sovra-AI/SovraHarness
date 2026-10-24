//! Real protocol and HTTP integration, with local deterministic model servers.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};
use sovra_core::{AgentConfig, Command, Event, models::*};
use sovra_providers::{
    CompatibleProvider, CredentialStore, ModelRepository, OllamaProvider, OpenRouterProvider,
    SavedModels,
};
use sovra_runtime::{Client, ModelsConfig};

#[derive(Default)]
struct TestCredentials(Mutex<BTreeMap<String, ModelSecret>>);
impl CredentialStore for TestCredentials {
    fn get(&self, id: &str) -> Result<ModelSecret, ModelError> {
        Ok(self.0.lock().unwrap().get(id).cloned().unwrap_or_default())
    }
    fn set(&self, id: &str, key: &ModelSecret) -> Result<(), ModelError> {
        self.0.lock().unwrap().insert(id.into(), key.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), ModelError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Request {
    path: String,
    headers: String,
    body: Value,
}
struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    tail: Option<(Arc<Mutex<std::sync::mpsc::Receiver<()>>>, String)>,
}
impl Reply {
    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: value.to_string(),
            tail: None,
        }
    }
}
struct Server {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Request>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(handler: impl Fn(&Request) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let thread = std::thread::spawn(move || {
            for socket in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let mut socket = socket.unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    let count = match socket.read(&mut chunk) {
                        Ok(0) | Err(_) => break None,
                        Ok(n) => n,
                    };
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(end + 4);
                    }
                    assert!(bytes.len() < 1024 * 1024);
                };
                let Some(header_end) = header_end else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                let length = headers
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map_or(0, |(_, v)| v.trim().parse::<usize>().unwrap());
                while bytes.len() < header_end + length {
                    let n = socket.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let body = serde_json::from_slice(&bytes[header_end..]).unwrap_or(Value::Null);
                let request = Request {
                    path,
                    headers,
                    body,
                };
                let reply = handler(&request);
                observed.lock().unwrap().push(request);
                let status = if reply.status == 200 { "OK" } else { "Error" };
                let response = format!(
                    "HTTP/1.1 {} {status}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    reply.status,
                    reply.content_type,
                    reply.body.len() + reply.tail.as_ref().map_or(0, |(_, tail)| tail.len()),
                    reply.body
                );
                let _ = socket.write_all(response.as_bytes());
                if let Some((gate, tail)) = reply.tail
                    && gate
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .is_ok()
                {
                    let _ = socket.write_all(tail.as_bytes());
                }
            }
        });
        Self {
            address,
            stop,
            requests,
            thread: Some(thread),
        }
    }
    fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

struct Fixture {
    path: std::path::PathBuf,
    repo: ModelRepository,
}
impl Fixture {
    fn new(connection: ModelConnection, secret: Option<&str>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sovra-model-integration-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let credentials = Arc::new(TestCredentials::default());
        if let Some(secret) = secret {
            credentials
                .set(&connection.id, &ModelSecret(secret.into()))
                .unwrap();
        }
        let repo = ModelRepository::new(path.clone(), credentials, BTreeMap::new());
        repo.save(&SavedModels {
            version: 1,
            connections: vec![connection],
            preferences: ModelPreferences::default(),
        })
        .unwrap();
        Self { path, repo }
    }
    fn start(&self) -> Client {
        sovra_runtime::start_with_models(
            "test",
            AgentConfig::default(),
            None,
            Some(ModelsConfig {
                repository: self.repo.clone(),
                providers: vec![
                    Arc::new(OpenRouterProvider),
                    Arc::new(CompatibleProvider),
                    Arc::new(OllamaProvider),
                ],
            }),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

async fn event(client: &Client, predicate: impl Fn(&Event) -> bool) -> Event {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = client.events().recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
        }
    })
    .await
    .expect("expected event within 20 seconds")
}
async fn catalog(client: &Client) -> CatalogSnapshot {
    match event(client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::CatalogLoaded { cached: false, .. })
        )
    })
    .await
    {
        Event::Models(ModelEvent::CatalogLoaded { snapshot, .. }) => snapshot,
        _ => unreachable!(),
    }
}
fn send(client: &Client, command: ModelCommand) {
    client.send(Command::Models(command)).unwrap();
}
async fn stop(client: Client) {
    client.send(Command::Shutdown).unwrap();
    event(&client, |e| matches!(e, Event::Stopped)).await;
}

#[tokio::test]
async fn openrouter_fetches_all_modalities_and_retains_cache_after_failure() {
    let fail = Arc::new(AtomicBool::new(false));
    let server_fail = fail.clone();
    let server = Server::new(move |request| {
        assert_eq!(request.path, "/api/v1/models?output_modalities=all");
        if server_fail.load(Ordering::SeqCst) {
            return Reply {
                status: 503,
                tail: None,
                content_type: "application/json",
                body: "{}".into(),
            };
        }
        Reply::json(&json!({"data":[
            {"id":"a/text","architecture":{"output_modalities":["text"]}},
            {"id":"b/embed","architecture":{"output_modalities":["embeddings"]}},
            {"id":"c/video","architecture":{"output_modalities":["video"]}}
        ]}))
    });
    let mut connection = ModelConnection::new("router", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    let snapshot = catalog(&client).await;
    assert_eq!(snapshot.models.len(), 3);
    assert_eq!(snapshot.models[2].output_modalities, ["video"]);
    send(&client, ModelCommand::Load);
    let replay = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::CatalogLoaded { .. }))
    })
    .await;
    assert!(
        matches!(
            replay,
            Event::Models(ModelEvent::CatalogLoaded { cached: false, .. })
        ),
        "a fresh catalog must stay fresh when reopening Models"
    );
    fail.store(true, Ordering::SeqCst);
    send(
        &client,
        ModelCommand::Refresh {
            connection_id: "router".into(),
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::CatalogFailed { .. }))
    })
    .await;
    send(&client, ModelCommand::Load);
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::CatalogFailed { .. }))
    })
    .await;
    assert_eq!(
        fixture
            .repo
            .load_catalog("router")
            .unwrap()
            .unwrap()
            .models
            .len(),
        3
    );
    stop(client).await;
}

#[tokio::test]
async fn selected_custom_model_routes_real_chat_and_restores_preferences() {
    custom_chat_with_authentication(Authentication::Bearer).await;
}

#[tokio::test]
async fn local_custom_chat_sends_no_authorization_header() {
    custom_chat_with_authentication(Authentication::None).await;
}

#[tokio::test]
async fn custom_chat_supports_api_key_headers_without_bearer() {
    custom_chat_with_authentication(Authentication::ApiKey).await;
}

async fn custom_chat_with_authentication(authentication: Authentication) {
    let server = Server::new(move |request| {
        let headers = request.headers.to_ascii_lowercase();
        match authentication {
            Authentication::Bearer => {
                assert!(headers.contains("authorization: bearer fake-test-key"));
            }
            Authentication::ApiKey => {
                assert!(headers.contains("api-key: fake-test-key"));
                assert!(!headers.contains("authorization:"));
            }
            Authentication::None => {
                assert!(!headers.contains("authorization:"));
                assert!(!headers.contains("api-key:"));
            }
        }
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"local-chat"}]}));
        }
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.body["model"], "local-chat");
        if request.body["stream"] == true {
            Reply {
                status: 200,
                content_type: "text/event-stream",
                tail: None,
                body: format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"local-chat","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello from custom model"},"finish_reason":null}]}),
                    json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"local-chat","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
                ),
            }
        } else {
            Reply::json(
                &json!({"id":"c","object":"chat.completion","created":1,"model":"local-chat","choices":[{"index":0,"message":{"role":"assistant","content":"Hello from custom model"},"finish_reason":"stop"}]}),
            )
        }
    });
    let mut connection = ModelConnection::new("custom", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.authentication = authentication;
    connection.api_key_header = "api-key".into();
    connection.has_credential = authentication != Authentication::None;
    let fixture = Fixture::new(
        connection,
        (authentication != Authentication::None).then_some("fake-test-key"),
    );
    let client = fixture.start();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "custom".into(),
        model_id: "local-chat".into(),
    };
    send(
        &client,
        ModelCommand::SetFavorite {
            key: key.clone(),
            favorite: true,
        },
    );
    event(&client,|e| matches!(e,Event::Models(ModelEvent::PreferencesChanged(p)) if p.favorites.contains(&key))).await;
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client,|e| matches!(e,Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref()==Some(&key))).await;
    client
        .send(Command::RunPrompt {
            session_id: "integration".into(),
            prompt: "Hello".into(),
        })
        .unwrap();
    let reply = event(&client, |e| {
        matches!(e, Event::AgentChunk { .. } | Event::AgentError { .. })
    })
    .await;
    assert!(
        matches!(reply,Event::AgentChunk { text,.. } if text.contains("Hello from custom model"))
    );
    event(&client, |e| matches!(e, Event::AgentCompleted { .. })).await;
    assert!(
        !std::fs::read_to_string(fixture.path.join("connections.json"))
            .unwrap()
            .contains("fake-test-key")
    );
    stop(client).await;
    let restarted = fixture.start();
    event(&restarted,|e|matches!(e,Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref()==Some(&key))).await;
    assert!(
        fixture
            .repo
            .load()
            .unwrap()
            .preferences
            .favorites
            .contains(&key)
    );
    stop(restarted).await;
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.path == "/v1/chat/completions")
    );
}

#[tokio::test]
async fn ollama_discovery_enriches_metadata_and_pull_streams_progress() {
    let downloaded = Arc::new(AtomicBool::new(false));
    let installed = downloaded.clone();
    let server = Server::new(move |request| match request.path.as_str() {
        "/api/tags" => {
            let mut models = vec![
                json!({"model":"local:8b","size":5000,"details":{"family":"qwen","parameter_size":"8B","quantization_level":"Q4_K_M"}}),
            ];
            if installed.load(Ordering::SeqCst) {
                models.push(json!({"model":"new:4b"}));
            }
            Reply::json(&json!({"models":models}))
        }
        "/api/show" => Reply::json(
            &json!({"capabilities":["completion","tools","vision"],"model_info":{"qwen.context_length":128_000}}),
        ),
        "/api/pull" => {
            assert_eq!(request.body["model"], "new:4b");
            assert_eq!(request.body["stream"], true);
            installed.store(true, Ordering::SeqCst);
            Reply { status:200,tail:None,content_type:"application/x-ndjson",body:"{\"status\":\"pulling\",\"completed\":50,\"total\":100}\n{\"status\":\"success\"}".into() }
        }
        other => panic!("unexpected path {other}"),
    });
    let mut connection = ModelConnection::new("ollama", ProviderKind::Ollama);
    connection.base_url = server.url();
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    let snapshot = catalog(&client).await;
    assert_eq!(snapshot.models[0].context_length, Some(128_000));
    assert!(snapshot.models[0].supports("tools"));
    send(
        &client,
        ModelCommand::PullOllama {
            connection_id: "ollama".into(),
            model: "new:4b".into(),
        },
    );
    event(
        &client,
        |e| matches!(e,Event::Models(ModelEvent::PullProgress(Some(p))) if p.contains("50%")),
    )
    .await;
    assert_eq!(catalog(&client).await.models.len(), 2);
    stop(client).await;
}

#[tokio::test]
async fn changing_a_credentialed_endpoint_requires_a_new_key() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("custom", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.authentication = Authentication::Bearer;
    connection.has_credential = true;
    let fixture = Fixture::new(connection.clone(), Some("fake-test-key"));
    let client = fixture.start();
    catalog(&client).await;
    connection.base_url = "https://different.example/v1".into();
    send(
        &client,
        ModelCommand::TestConnection {
            connection,
            credential: CredentialChange::Keep,
        },
    );
    let failure = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Error(_)))
    })
    .await;
    assert!(
        matches!(failure,Event::Models(ModelEvent::Error(message)) if message.contains("Re-enter"))
    );
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    stop(client).await;
}

#[tokio::test]
async fn real_tool_execution_delivers_agui_components_to_client() {
    let called = AtomicBool::new(false);
    let server = Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"ui-model", "supported_parameters":["tools"], "output_modalities":["text"]}]}),
            );
        }
        assert_eq!(request.path, "/v1/chat/completions");
        let first = !called.swap(true, Ordering::SeqCst);
        let args = json!({"title":"Report", "content":"Actual tool content", "actions":[{"label":"Continue", "action_id":"next"}]}).to_string();
        let tool_call = json!({"index":0, "id":"call-1", "type":"function", "function":{"name":"render_card", "arguments":args}});
        let (message, finish) = if first {
            assert!(
                request.body["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| tool["function"]["name"] == "render_card")
            );
            (
                json!({"role":"assistant", "tool_calls":[tool_call]}),
                "tool_calls",
            )
        } else {
            assert!(
                request.body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|message| message["role"] == "tool")
            );
            (
                json!({"role":"assistant", "content":"Here is your report."}),
                "stop",
            )
        };
        if request.body["stream"] == true {
            Reply {
                status: 200,
                content_type: "text/event-stream",
                tail: None,
                body: format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"id":"c", "object":"chat.completion.chunk", "created":1, "model":"ui-model", "choices":[{"index":0, "delta":message, "finish_reason":null}]}),
                    json!({"id":"c", "object":"chat.completion.chunk", "created":1, "model":"ui-model", "choices":[{"index":0, "delta":{}, "finish_reason":finish}]})
                ),
            }
        } else {
            Reply::json(
                &json!({"id":"c", "object":"chat.completion", "created":1, "model":"ui-model", "choices":[{"index":0, "message":message, "finish_reason":finish}]}),
            )
        }
    });
    let mut connection = ModelConnection::new("native-ui", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "native-ui".into(),
        model_id: "ui-model".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key))).await;
    client
        .send(Command::RunPrompt {
            session_id: "native-ui-session".into(),
            prompt: "Show a report".into(),
        })
        .unwrap();
    let rendered = event(&client, |event| {
        matches!(
            event,
            Event::AgUiSurface { .. } | Event::AgentError { .. } | Event::AgentCompleted { .. }
        )
    })
    .await;
    let Event::AgUiSurface {
        session_id,
        widget: sovra_core::AgUiWidget::Components { components, .. },
    } = rendered
    else {
        panic!("expected actual native components, got {rendered:?}")
    };
    assert_eq!(session_id, "native-ui-session");
    let sovra_core::UiComponent::Card {
        title, children, ..
    } = &components[0]
    else {
        panic!("expected card")
    };
    assert_eq!(title, "Report");
    assert!(
        matches!(&children[0], sovra_core::UiComponent::Text { text, .. } if text == "Actual tool content")
    );
    assert!(
        matches!(&children[1], sovra_core::UiComponent::Button { action_id, .. } if action_id == "next")
    );
    event(&client, |e| matches!(e, Event::AgentCompleted { .. })).await;
    stop(client).await;
}

#[tokio::test]
async fn saved_key_metadata_and_secret_survive_restart_replace_and_remove() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection.clone(), None);
    let client = fixture.start();
    catalog(&client).await;
    for value in ["initial-test-key", "replacement-test-key"] {
        send(
            &client,
            ModelCommand::SaveConnection {
                connection: connection.clone(),
                credential: CredentialChange::Replace(ModelSecret(value.into())),
            },
        );
        event(&client, |e| {
            matches!(e, Event::Models(ModelEvent::ConnectionSaved { .. }))
        })
        .await;
        let saved = fixture.repo.load().unwrap();
        connection = saved.connections[0].clone();
        assert!(connection.has_credential);
        assert_eq!(fixture.repo.credential(&connection).unwrap().0, value);
        assert!(
            !std::fs::read_to_string(fixture.path.join("connections.json"))
                .unwrap()
                .contains(value)
        );
    }
    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { connections, .. }) if connections[0].has_credential)
    );
    assert_eq!(
        fixture.repo.credential(&connection).unwrap().0,
        "replacement-test-key"
    );
    send(
        &client,
        ModelCommand::SaveConnection {
            connection,
            credential: CredentialChange::Remove,
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::ConnectionSaved { .. }))
    })
    .await;
    let saved = fixture.repo.load().unwrap();
    assert!(!saved.connections[0].has_credential);
    assert_eq!(
        fixture.repo.credential(&saved.connections[0]).unwrap().0,
        ""
    );
    stop(client).await;
}

#[tokio::test]
async fn conversational_reply_arrives_before_the_server_finishes_without_cards_or_routing() {
    let (release, gate) = std::sync::mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let server = Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"chat-model", "supported_parameters":["tools"]}]}),
            );
        }
        assert_eq!(request.body["stream"], true);
        assert_ne!(request.body["tool_choice"], "required");
        // ADK 2.3 represents its instruction preamble as user Content; inspect
        // the actual outbound context rather than assuming a provider role.
        let instructions = request.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["content"].to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            instructions.contains("conversational assistant"),
            "{instructions}"
        );
        assert!(instructions.contains("Native UI tools are optional"));
        let chunk = |text: &str, finish: Value| {
            format!(
                "data: {}\n\n",
                json!({
                    "id":"streaming", "object":"chat.completion.chunk", "created":1,
                    "model":"chat-model", "choices":[{"index":0,
                        "delta":{"role":"assistant","content":text}, "finish_reason":finish}]
                })
            )
        };
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body: chunk("Hello **", Value::Null),
            tail: Some((
                gate.clone(),
                format!(
                    "{}{}data: [DONE]\n\n",
                    chunk("world**!", Value::Null),
                    chunk("", json!("stop"))
                ),
            )),
        }
    });
    let mut connection = ModelConnection::new("streaming", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "streaming".into(),
        model_id: "chat-model".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key))).await;
    client
        .send(Command::RunPrompt {
            session_id: "live-stream".into(),
            prompt: "Hi!".into(),
        })
        .unwrap();
    let events = client.events();
    let mut text = String::new();
    let mut responder = None;
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                Event::AgentResponseStarted { session_id, model } => {
                    assert_eq!(session_id, "live-stream");
                    assert!(responder.replace(model).is_none());
                }
                Event::AgentChunk { text, .. } => break text,
                Event::Models(_) | Event::Ready { .. } => {}
                event => panic!("unexpected event before first text: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        responder.unwrap(),
        sovra_core::ResponseModel {
            model_id: "chat-model".into(),
            name: "chat-model".into(),
            provider: "OpenAI compatible".into(),
        }
    );
    assert_eq!(first, "Hello **");
    text.push_str(&first);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await.unwrap() {
                Event::AgentChunk { text: chunk, .. } => text.push_str(&chunk),
                Event::AgentCompleted { .. } => break,
                event => panic!("basic chat must not emit cards or routing: {event:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(text, "Hello **world**!");
    stop(client).await;
}

#[tokio::test]
async fn appearance_choice_survives_restart() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    send(
        &client,
        ModelCommand::SetAppearance {
            appearance: Appearance::Light,
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.appearance == Appearance::Light)
    })
    .await;
    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.appearance == Appearance::Light)
    );
    stop(client).await;
}
