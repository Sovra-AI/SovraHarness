//! Real protocol and HTTP integration, with local deterministic model servers.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeMap;
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

#[path = "models/live_web_search.rs"]
mod live_web_search;

#[path = "models/memory.rs"]
mod memory;

#[path = "models/semantic.rs"]
mod semantic;

#[path = "models/summaries.rs"]
mod summaries;

#[path = "models/telegram.rs"]
mod telegram;

#[path = "models/whatsapp.rs"]
mod whatsapp;

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

#[path = "../../../tests/support/model_http.rs"]
mod http;
use http::{Reply, Request, Server};

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
            context: None,
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
            context: None,
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

/// Regression: `OpenRouter` lists models without a key, so a mistyped key used to
/// save as "connected" and only fail on the first message ("User not found").
#[tokio::test]
async fn a_key_openrouter_rejects_is_not_saved() {
    let server = Server::new(|request| {
        if request.path.ends_with("/auth/key") {
            return Reply {
                status: 401,
                content_type: "application/json",
                body: json!({"error":{"message":"User not found.","code":401}}).to_string(),
                tail: None,
            };
        }
        Reply::json(&json!({"data":[]}))
    });
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection.clone(), None);
    let client = fixture.start();
    catalog(&client).await;
    send(
        &client,
        ModelCommand::SaveConnection {
            connection: connection.clone(),
            credential: CredentialChange::Replace(ModelSecret("sk-or-v1-truncated".into())),
        },
    );
    let failure = event(&client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::Error(_) | ModelEvent::ConnectionSaved { .. })
        )
    })
    .await;
    assert!(
        matches!(&failure, Event::Models(ModelEvent::Error(message)) if message.contains("didn't accept this key")),
        "{failure:?}"
    );
    let saved = fixture.repo.load().unwrap();
    assert!(saved.connections.iter().all(|c| !c.has_credential));
    assert_eq!(fixture.repo.credential(&connection).unwrap().0, "");
    stop(client).await;
}

/// Regression: a settings file that records a saved key the credential store no longer
/// holds (a reset keychain, or a dev build reading its own store) used to report
/// "ready" while every message bounced back to setup.
#[tokio::test]
async fn a_saved_key_missing_from_the_store_asks_for_the_key_then_recovers() {
    let server = Server::new(|_| {
        Reply::json(&json!({"data":[
            {"id":"a/chat","architecture":{"output_modalities":["text"]},
             "pricing":{"prompt":"0","completion":"0"}}
        ]}))
    });
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    connection.has_credential = true;
    let fixture = Fixture::new(connection.clone(), None);
    let key = ModelKey {
        connection_id: "openrouter".into(),
        model_id: "a/chat".into(),
    };
    let mut saved = fixture.repo.load().unwrap();
    saved.preferences.active = Some(key.clone());
    fixture.repo.save(&saved).unwrap();

    let client = fixture.start();
    let mut state = ModelsState::default();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    let Event::Models(loaded) = loaded else {
        unreachable!()
    };
    state.apply(&loaded);
    assert!(!state.connections[0].has_credential);
    assert!(matches!(
        state.setup_stage(),
        SetupStage::NeedsModelKey { .. }
    ));
    catalog(&client).await;
    // With the catalog in, the runtime retries the chosen model and still has no key.
    event(&client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::ActiveModelStatus { problem: Some(_) })
        )
    })
    .await;

    send(
        &client,
        ModelCommand::SaveConnection {
            connection: state.connections[0].clone(),
            credential: CredentialChange::Replace(ModelSecret("restored-key".into())),
        },
    );
    // The model is ready before the UI hears the key was saved, so a waiting
    // message sent at that moment reaches it.
    let ready = event(&client, |e| {
        matches!(
            e,
            Event::Models(ModelEvent::ActiveModelStatus { .. } | ModelEvent::Loaded { .. })
        )
    })
    .await;
    assert!(
        matches!(
            ready,
            Event::Models(ModelEvent::ActiveModelStatus { problem: None })
        ),
        "activation must precede the saved-key announcement: {ready:?}"
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
            context: None,
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
                Event::Models(_) | Event::Ready { .. } | Event::RouteEvaluated { .. } => {}
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

#[tokio::test]
async fn language_defaults_to_english_and_survives_restart() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.language == UiLanguage::default() && preferences.language.tag() == "en" && !preferences.language.is_rtl())
    );
    // Language is stored with preferences. The model catalog is a separate
    // refresh and must not gate saving or reloading the chosen tag.
    send(
        &client,
        ModelCommand::SetLanguage {
            language: UiLanguage::from_tag("tr"),
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.language.tag() == "tr" && !p.language.is_rtl())
    })
    .await;
    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.language.tag() == "tr" && !preferences.language.is_rtl())
    );
    send(
        &client,
        ModelCommand::SetLanguage {
            language: UiLanguage::english(),
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.language == UiLanguage::english() && !p.language.is_rtl())
    })
    .await;
    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.language == UiLanguage::english() && !preferences.language.is_rtl())
    );
    stop(client).await;
}

#[tokio::test]
async fn interface_scale_survives_restart() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    let scale = InterfaceScale::from_percent(125);
    send(&client, ModelCommand::SetInterfaceScale { scale });
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.interface_scale == scale)
    })
    .await;
    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.interface_scale == scale)
    );
    stop(client).await;
}

#[tokio::test]
async fn onboarding_completion_and_replay_survive_restart() {
    let server = Server::new(|_| Reply::json(&json!({"data":[]})));
    let mut connection = ModelConnection::new("openrouter", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    let fixture = Fixture::new(connection, None);
    for completed in [true, false] {
        let client = fixture.start();
        event(&client, |e| {
            matches!(e, Event::Models(ModelEvent::Loaded { .. }))
        })
        .await;
        send(&client, ModelCommand::SetOnboardingCompleted { completed });
        event(&client, |e| {
            matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.onboarding_completed == completed)
        }).await;
        stop(client).await;
        let client = fixture.start();
        let loaded = event(&client, |e| {
            matches!(e, Event::Models(ModelEvent::Loaded { .. }))
        })
        .await;
        assert!(
            matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.onboarding_completed == completed)
        );
        stop(client).await;
    }
}

struct AttachmentFixture;
impl sovra_core::files::FileStore for AttachmentFixture {
    fn prepare(
        &self,
        id: &str,
        _path: &std::path::Path,
    ) -> Result<sovra_core::files::Attachment, sovra_core::files::FileError> {
        self.load(id).map(|file| file.attachment)
    }
    fn load(
        &self,
        id: &str,
    ) -> Result<sovra_core::files::FileContent, sovra_core::files::FileError> {
        use sovra_core::files::{Attachment, FileContent, FileError, MediaPart, ModelPurpose};
        let (purpose, mime, text, data) = match id {
            "test-image" => (ModelPurpose::Images, "image/png", "", vec![137, 80, 78, 71]),
            "test-document" => (
                ModelPurpose::Documents,
                "text/plain",
                "The garden invoice total is 93 dirhams.",
                Vec::new(),
            ),
            _ => return Err(FileError("Please add this missing file again.".into())),
        };
        Ok(FileContent {
            raw: None,
            attachment: Attachment {
                id: id.into(),
                name: format!("{id}.file"),
                mime_type: mime.into(),
                size_bytes: 42,
                purpose,
                sections: 1,
                note: None,
                thumbnail: Vec::new(),
            },
            text: text.into(),
            media: if data.is_empty() {
                Vec::new()
            } else {
                vec![MediaPart {
                    duration_millis: None,
                    mime_type: mime.into(),
                    data,
                }]
            },
        })
    }
    fn discard(&self, _id: &str) -> Result<(), sovra_core::files::FileError> {
        Ok(())
    }
}

fn attachment_reply(model: &str) -> Reply {
    Reply {
        status: 200,
        content_type: "text/event-stream",
        tail: None,
        body: format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"id":"files","object":"chat.completion.chunk","created":1,"model":model,"choices":[{"index":0,"delta":{"role":"assistant","content":"The total is 93 dirhams."},"finish_reason":null}]}),
            json!({"id":"files","object":"chat.completion.chunk","created":1,"model":model,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        ),
    }
}

#[tokio::test]
async fn purpose_assignment_keeps_chat_model_and_sends_real_images_and_document_evidence() {
    use sovra_core::files::ModelPurpose;
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"chat"},{"id":"vision"}]}));
        }
        attachment_reply(request.body["model"].as_str().unwrap())
    });
    let mut connection = ModelConnection::new("files", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.manual_models = vec![ModelInfo {
        id: "vision".into(),
        name: "Picture reader".into(),
        input_modalities: vec!["text".into(), "image".into()],
        output_modalities: vec!["text".into()],
        manual: true,
        ..ModelInfo::default()
    }];
    let fixture = Fixture::new(connection, None);
    let client = sovra_runtime::start_with_files(
        "test",
        AgentConfig::default(),
        None,
        Some(ModelsConfig {
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(CompatibleProvider)],
        }),
        Some(Arc::new(AttachmentFixture)),
    )
    .unwrap();
    catalog(&client).await;
    let chat = ModelKey {
        connection_id: "files".into(),
        model_id: "chat".into(),
    };
    let vision = ModelKey {
        connection_id: "files".into(),
        model_id: "vision".into(),
    };
    send(&client, ModelCommand::Activate { key: chat.clone() });
    event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(preferences)) if preferences.active.as_ref() == Some(&chat))).await;
    send(
        &client,
        ModelCommand::AssignPurpose {
            purpose: ModelPurpose::Images,
            key: Some(chat.clone()),
        },
    );
    event(&client, |event| {
        matches!(event, Event::Models(ModelEvent::Error(_)))
    })
    .await;
    assert!(fixture.repo.load().unwrap().preferences.purposes.is_empty());
    send(
        &client,
        ModelCommand::AssignPurpose {
            purpose: ModelPurpose::Images,
            key: Some(vision.clone()),
        },
    );
    let changed = event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(preferences)) if preferences.purposes.get(&ModelPurpose::Images) == Some(&vision))).await;
    assert!(
        matches!(changed, Event::Models(ModelEvent::PreferencesChanged(preferences)) if preferences.active == Some(chat))
    );
    for (session_id, id) in [
        ("picture-chat", "test-image"),
        ("document-chat", "test-document"),
    ] {
        client
            .send(Command::RunWithFiles {
                context: None,
                session_id: session_id.into(),
                prompt: "What is the invoice total?".into(),
                attachment_ids: vec![id.into()],
            })
            .unwrap();
        let completed = event(&client, |event| {
            matches!(
                event,
                Event::AgentCompleted { .. } | Event::AgentError { .. }
            )
        })
        .await;
        assert!(
            matches!(completed, Event::AgentCompleted { .. }),
            "{completed:?}"
        );
    }
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "picture-chat".into(),
            prompt: "Explain that again.".into(),
        })
        .unwrap();
    event(&client, |event| {
        matches!(event, Event::AgentCompleted { .. })
    })
    .await;
    {
        let requests = server.requests.lock().unwrap();
        let requests: Vec<_> = requests
            .iter()
            .filter(|request| request.path.ends_with("chat/completions"))
            .collect();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].body["model"], "vision");
        let picture = requests[0].body.to_string();
        assert!(
            picture.contains("data:image/png;base64,iVBORw=="),
            "{picture}"
        );
        assert!(
            requests[1]
                .body
                .to_string()
                .contains("garden invoice total is 93")
        );
        assert_eq!(requests[2].body["model"], "vision");
    }
    stop(client).await;
    assert_eq!(
        fixture
            .repo
            .load()
            .unwrap()
            .preferences
            .purposes
            .get(&ModelPurpose::Images),
        Some(&vision)
    );
}

#[tokio::test]
async fn uploads_report_results_and_missing_storage_explains_how_to_recover() {
    use sovra_core::files::{FileCommand, FilePrepared};
    let client = sovra_runtime::start_with_files(
        "test",
        AgentConfig::default(),
        None,
        None,
        Some(Arc::new(AttachmentFixture)),
    )
    .unwrap();
    client
        .send(Command::Files(FileCommand::Prepare {
            session_id: "draft".into(),
            id: "test-image".into(),
            path: "chosen.png".into(),
        }))
        .unwrap();
    let ready = event(&client, |event| matches!(event, Event::FilePrepared(_))).await;
    assert!(matches!(
        ready,
        Event::FilePrepared(FilePrepared { result: Ok(_), .. })
    ));
    client
        .send(Command::Files(FileCommand::Prepare {
            session_id: "draft".into(),
            id: "gone".into(),
            path: "gone.txt".into(),
        }))
        .unwrap();
    let failed = event(&client, |event| matches!(event, Event::FilePrepared(_))).await;
    assert!(matches!(
        failed,
        Event::FilePrepared(FilePrepared { result: Err(_), .. })
    ));
    stop(client).await;
}
