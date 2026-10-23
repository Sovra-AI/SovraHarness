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
}
impl Reply {
    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body: value.to_string(),
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
                    reply.body.len(),
                    reply.body
                );
                let _ = socket.write_all(response.as_bytes());
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
            Reply { status:200,content_type:"application/x-ndjson",body:"{\"status\":\"pulling\",\"completed\":50,\"total\":100}\n{\"status\":\"success\"}".into() }
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
