//! Plain build requests and code explanations through the typed-message dispatch.
//!
//! The scripted model never leaves this process. A garden-club page must take
//! the file-writing run. An explanation must stay a normal answer.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;

use serde_json::{Value, json};
use sovra_core::models::{
    Authentication, ModelCommand, ModelConnection, ModelEvent, ModelKey, ModelSecret, ProviderKind,
};
use sovra_core::{
    AgentConfig, Command, Episode, EpisodeId, Event, FactId, MemoryCandidate, MemoryClaim,
    MemoryError, MemoryExtractedChunk, MemoryInput, MemoryJob, MemoryPolicy, MemoryStore,
    NewEpisode, NewFact, RecallQuery, RecalledFact, Timestamp,
};
use sovra_providers::{CompatibleProvider, CredentialStore, ModelRepository, SavedModels};
use sovra_runtime::{Client, ModelsConfig, RuntimeServices};

#[path = "../../../tests/support/model_http.rs"]
mod http;
use http::{Reply, Request, Server};

const PAGE: &str = "<title>Garden club</title><style>body{color:green}</style><h1>Tuesday</h1><script>console.log('meet')</script>";
const BUILD: &str = "Build a garden-club page with the Tuesday meeting time";
const EXPLAIN: &str = "What does this code mean?";
const NOTE: &str = "Your garden club page is ready to open.";

#[derive(Default)]
struct TestCredentials(Mutex<BTreeMap<String, ModelSecret>>);
impl CredentialStore for TestCredentials {
    fn get(&self, id: &str) -> Result<ModelSecret, sovra_core::models::ModelError> {
        Ok(self.0.lock().unwrap().get(id).cloned().unwrap_or_default())
    }
    fn set(&self, id: &str, key: &ModelSecret) -> Result<(), sovra_core::models::ModelError> {
        self.0.lock().unwrap().insert(id.into(), key.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), sovra_core::models::ModelError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

struct Fixture {
    path: std::path::PathBuf,
    repo: ModelRepository,
}
impl Fixture {
    fn new(connection: ModelConnection) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sovra-coding-route-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let repo = ModelRepository::new(
            path.clone(),
            Arc::new(TestCredentials::default()),
            BTreeMap::new(),
        );
        repo.save(&SavedModels {
            version: 1,
            connections: vec![connection],
            preferences: sovra_core::models::ModelPreferences::default(),
        })
        .unwrap();
        Self { path, repo }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Default)]
struct Library {
    saved: Mutex<Vec<sovra_core::artifacts::Artifact>>,
}

#[async_trait::async_trait]
impl MemoryStore for Library {
    async fn health(&self) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn memory_policy(&self) -> Result<MemoryPolicy, MemoryError> {
        Ok(MemoryPolicy {
            enabled: false,
            generation: 0,
        })
    }
    async fn set_memory_enabled(&self, _: bool) -> Result<MemoryPolicy, MemoryError> {
        self.memory_policy().await
    }
    async fn enqueue_memory(&self, _: MemoryInput) -> Result<EpisodeId, MemoryError> {
        Err(MemoryError::Backend(
            "unused in the coding route test".into(),
        ))
    }
    async fn claim_memory(&self, _: Timestamp) -> Result<Option<MemoryClaim>, MemoryError> {
        Ok(None)
    }
    async fn save_memory_extraction(
        &self,
        _: &MemoryClaim,
        _: &MemoryExtractedChunk,
        _: Timestamp,
    ) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn complete_memory(
        &self,
        _: &MemoryClaim,
        _: Vec<MemoryCandidate>,
        _: &str,
        _: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        Ok(Vec::new())
    }
    async fn fail_memory(&self, _: &MemoryClaim, _: &str, _: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn memory_job(&self, id: &EpisodeId) -> Result<MemoryJob, MemoryError> {
        Err(MemoryError::Backend(format!("no job for {}", id.0)))
    }
    async fn record_episode(&self, _: NewEpisode) -> Result<EpisodeId, MemoryError> {
        Err(MemoryError::Backend(
            "unused in the coding route test".into(),
        ))
    }
    async fn episode(&self, _: &EpisodeId) -> Result<Episode, MemoryError> {
        Err(MemoryError::Backend(
            "unused in the coding route test".into(),
        ))
    }
    async fn recent_episodes(&self, _: &str, _: usize) -> Result<Vec<Episode>, MemoryError> {
        Ok(Vec::new())
    }
    async fn save_fact(&self, _: NewFact) -> Result<FactId, MemoryError> {
        Err(MemoryError::Backend(
            "unused in the coding route test".into(),
        ))
    }
    async fn invalidate_fact(&self, _: &FactId, _: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn recall(&self, _: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        Ok(Vec::new())
    }
    async fn save_artifact(
        &self,
        artifact: &sovra_core::artifacts::Artifact,
    ) -> Result<(), MemoryError> {
        self.saved.lock().unwrap().push(artifact.clone());
        Ok(())
    }
    async fn list_artifacts(&self) -> Result<Vec<sovra_core::artifacts::Artifact>, MemoryError> {
        Ok(self.saved.lock().unwrap().clone())
    }
}

struct RouteSeen {
    garden_offered_write: bool,
    explain_offered_write: bool,
}

fn offers_write_file(body: &Value) -> bool {
    body["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "write_file")
    })
}

fn completion(message: &Value, finish: &str, stream: bool) -> Reply {
    if stream {
        Reply {
            status: 200,
            content_type: "text/event-stream",
            tail: None,
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"coding-model","choices":[{"index":0,"delta":message,"finish_reason":null}]}),
                json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"coding-model","choices":[{"index":0,"delta":{},"finish_reason":finish}]})
            ),
        }
    } else {
        Reply::json(&json!({
            "id": "c",
            "object": "chat.completion",
            "created": 1,
            "model": "coding-model",
            "choices": [{"index": 0, "message": message, "finish_reason": finish}]
        }))
    }
}

fn scripted(seen: Arc<Mutex<RouteSeen>>, garden_calls: Arc<AtomicUsize>) -> Server {
    Server::new(move |request: &Request| {
        assert!(!request.headers.is_empty(), "{}", request.path);
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{
                "id": "coding-model",
                "supported_parameters": ["tools"],
                "output_modalities": ["text"]
            }]}));
        }
        assert_eq!(request.path, "/v1/chat/completions");
        let text = request.body["messages"].to_string();
        let stream = request.body["stream"] == true;
        if text.contains(EXPLAIN) {
            if offers_write_file(&request.body) {
                seen.lock().unwrap().explain_offered_write = true;
            }
            let message =
                json!({"role":"assistant","content":"This loop repeats the steps inside it."});
            return completion(&message, "stop", stream);
        }
        if text.contains(BUILD) || text.contains("write_file") {
            let call = garden_calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                seen.lock().unwrap().garden_offered_write = offers_write_file(&request.body);
                let arguments = json!({"path":"index.html","content":PAGE}).to_string();
                let tool_call = json!({
                    "index": 0,
                    "id": "call-garden",
                    "type": "function",
                    "function": {"name": "write_file", "arguments": arguments}
                });
                let message = json!({"role":"assistant","tool_calls":[tool_call]});
                return completion(&message, "tool_calls", stream);
            }
            let message = json!({"role":"assistant","content":NOTE});
            return completion(&message, "stop", stream);
        }
        let message = json!({"role":"assistant","content":"Hello."});
        completion(&message, "stop", stream)
    })
}

async fn until(client: &Client, done: impl Fn(&Event) -> bool) -> Vec<Event> {
    tokio::time::timeout(Duration::from_secs(40), async {
        let mut seen = Vec::new();
        loop {
            let event = client.events().recv().await.unwrap();
            let finished = done(&event);
            seen.push(event);
            if finished {
                return seen;
            }
        }
    })
    .await
    .expect("timed out waiting for the coding route")
}

fn code_preview(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::AgUi {
                event:
                    sovra_core::AgUiEvent::ActivitySnapshot {
                        activity: sovra_core::LiveActivity::CodeWriting { content, .. },
                        ..
                    },
                ..
            } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn saved_cards(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::AgUiSurface {
                    widget: sovra_core::AgUiWidget::ArtifactSaved { .. },
                    ..
                }
            )
        })
        .count()
}

fn saved_bodies(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Artifacts(sovra_core::artifacts::ArtifactEvent::Saved(artifact)) => {
                Some(artifact.content.clone())
            }
            _ => None,
        })
        .collect()
}

fn closing_notes(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::AgUi {
                event: sovra_core::AgUiEvent::TextMessageContent { delta, .. },
                ..
            } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

fn assert_build(events: &[Event], library: &Library) {
    let previews = code_preview(events);
    assert_eq!(previews.len(), 2);
    assert!(previews.iter().all(|content| content == PAGE));
    let bodies = saved_bodies(events);
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0], PAGE);
    assert!(bodies[0].contains("<style"));
    assert!(bodies[0].contains("<script"));
    assert_eq!(saved_cards(events), 1);
    let notes = closing_notes(events);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0], NOTE);
    assert!(!notes[0].contains("```"));
    assert!(!notes[0].to_ascii_lowercase().contains("/coding"));
    assert!(!notes[0].to_ascii_lowercase().contains("slash"));
    let stored = library.saved.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].content, PAGE);
    assert_eq!(stored[0].kind, sovra_core::artifacts::ArtifactKind::Webpage);
    assert!(events.iter().any(|event| matches!(
        event,
        Event::AgUi {
            event: sovra_core::AgUiEvent::RunFinished { .. },
            ..
        }
    )));
}

fn assert_explain(events: &[Event], library: &Library, saved_before: usize) {
    assert_eq!(code_preview(events).len(), 0);
    assert_eq!(saved_cards(events), 0);
    assert_eq!(saved_bodies(events).len(), 0);
    assert_eq!(library.saved.lock().unwrap().len(), saved_before);
    // A normal answer is a chat chunk. The coding run's note is a text message.
    let answer: String = events
        .iter()
        .filter_map(|event| match event {
            Event::AgentChunk { text, .. } => Some(text.as_str()),
            Event::AgUi {
                event: sovra_core::AgUiEvent::TextMessageContent { delta, .. },
                ..
            } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        answer.contains("repeats the steps"),
        "explain answer: {answer}"
    );
    assert!(!answer.contains("```"));
    assert!(!answer.to_ascii_lowercase().contains("/coding"));
    assert!(!answer.to_ascii_lowercase().contains("slash"));
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::AgUi {
            event: sovra_core::AgUiEvent::RunError { message },
            ..
        } if message.contains("didn't write any files")
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::AgentCompleted { .. }))
    );
}

async fn exercise(client: &Client, library: &Library, seen: &Mutex<RouteSeen>) {
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "garden-build".into(),
            prompt: BUILD.into(),
        })
        .unwrap();
    let build = until(client, |event| {
        matches!(
            event,
            Event::AgUi {
                event: sovra_core::AgUiEvent::RunFinished { .. },
                ..
            } | Event::AgentError { .. }
                | Event::AgUi {
                    event: sovra_core::AgUiEvent::RunError { .. },
                    ..
                }
        )
    })
    .await;
    assert!(
        !build.iter().any(|event| matches!(
            event,
            Event::AgentError { .. }
                | Event::AgUi {
                    event: sovra_core::AgUiEvent::RunError { .. },
                    ..
                }
        )),
        "build failed: {build:?}"
    );
    assert_build(&build, library);
    assert!(seen.lock().unwrap().garden_offered_write);

    let saved_before = library.saved.lock().unwrap().len();
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "explain-code".into(),
            prompt: EXPLAIN.into(),
        })
        .unwrap();
    let explain = until(client, |event| {
        matches!(
            event,
            Event::AgentCompleted { .. } | Event::AgentError { .. }
        )
    })
    .await;
    assert_explain(&explain, library, saved_before);
    assert!(!seen.lock().unwrap().explain_offered_write);
}

#[tokio::test]
async fn plain_build_request_saves_the_page_and_an_explanation_does_not() {
    let seen = Arc::new(Mutex::new(RouteSeen {
        garden_offered_write: false,
        explain_offered_write: false,
    }));
    let server = scripted(Arc::clone(&seen), Arc::new(AtomicUsize::new(0)));
    let mut connection = ModelConnection::new("coding", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.authentication = Authentication::None;
    let fixture = Fixture::new(connection);
    let library = Arc::new(Library::default());
    let store = Arc::clone(&library);
    let memory: sovra_runtime::MemoryFactory =
        Box::new(move || Box::pin(async move { Ok(store as Arc<dyn MemoryStore>) }));
    let client = sovra_runtime::start_with_services(
        "coding-route",
        AgentConfig::default(),
        RuntimeServices {
            memory: Some(memory),
            models: Some(ModelsConfig {
                repository: fixture.repo.clone(),
                providers: vec![Arc::new(CompatibleProvider)],
            }),
            ..RuntimeServices::default()
        },
    )
    .unwrap();
    until(&client, |event| {
        matches!(
            event,
            Event::Models(ModelEvent::CatalogLoaded { cached: false, .. })
        )
    })
    .await;
    let key = ModelKey {
        connection_id: "coding".into(),
        model_id: "coding-model".into(),
    };
    client
        .send(Command::Models(ModelCommand::Activate { key: key.clone() }))
        .unwrap();
    until(&client, |event| {
        matches!(
            event,
            Event::Models(ModelEvent::PreferencesChanged(preferences))
                if preferences.active.as_ref() == Some(&key)
        )
    })
    .await;
    exercise(&client, &library, &seen).await;
    let recorded = server.requests.lock().unwrap().len();
    assert!(recorded >= 3, "recorded {recorded} model requests");
    client.send(Command::Shutdown).unwrap();
    until(&client, |event| matches!(event, Event::Stopped)).await;
}
