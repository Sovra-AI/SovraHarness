//! Composition wiring: real runtime, disk store and deterministic loopback model.
//! No application logic, private user data, credentials or live model service is used.
//! The ignored `live` module explicitly opts into synthetic live-model evaluation.

#![allow(clippy::unwrap_used, reason = "integration fixtures fail loudly")]

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sovra_core::{
    AgentConfig, ChatMessage, ChatThread, Command, ConversationContext, EpisodeKind, Event, Fact,
    MemoryAction, MemoryCommand, MemoryCorrection, MemoryEvent, MemoryJobState, MemoryListQuery,
    MemoryReviewResult, MemoryStatus, MemoryStore, models::*,
};
use sovra_providers::{CompatibleProvider, CredentialStore, ModelRepository, SavedModels};
use sovra_runtime::{Client, ModelsConfig};
use sovra_store_surreal::{StoreConfig, SurrealStore};

#[path = "../../../tests/support/model_http.rs"]
mod http;
use http::{Reply, Request, Server};

#[cfg(all(test, debug_assertions))]
#[path = "memory_lifecycle/live.rs"]
mod live;

#[path = "memory_lifecycle/knowledge.rs"]
mod knowledge;

const SOURCE: &str = "I prefer mint tea.";
const ORIGINAL: &str = "The user prefers mint tea.";
const CORRECTION: &str = "The user now prefers coffee.";
const PARAPHRASE: &str = "Mint tea remains my preference.";
const COLLEAGUE: &str = "My colleague Mira prefers rooibos tea with breakfast.";

struct NoCredentials;
impl CredentialStore for NoCredentials {
    fn get(&self, _: &str) -> Result<ModelSecret, ModelError> {
        Ok(ModelSecret::default())
    }
    fn set(&self, _: &str, _: &ModelSecret) -> Result<(), ModelError> {
        panic!("this fixture must not save credentials")
    }
    fn delete(&self, _: &str) -> Result<(), ModelError> {
        panic!("this fixture must not change credentials")
    }
}

struct Fixture {
    root: PathBuf,
    repository: ModelRepository,
    server: Server,
}
impl Fixture {
    fn new() -> Self {
        let server = Server::new(|request| {
            if request.path == "/v1/models" {
                return Reply::json(&json!({"data":[{"id":"memory-fixture"}]}));
            }
            let output = if is_extraction(request) {
                let data = request.body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|message| message["role"] == "user")
                    .unwrap();
                let data: Value = serde_json::from_str(data["content"].as_str().unwrap()).unwrap();
                if data["source"]["content"] == SOURCE {
                    json!({"memories":[{"content":ORIGINAL,"evidence":SOURCE,"entities":["user"],"valid_at":null,"replaces":null}]}).to_string()
                } else if data["source"]["content"] == COLLEAGUE {
                    json!({"memories":[
                        {"content":"Mira prefers rooibos tea with breakfast.","evidence":"Mira prefers rooibos tea with breakfast.","entities":["Mira"],"valid_at":null,"replaces":null},
                        {"content":"Mira is the user's colleague.","evidence":"My colleague Mira","entities":["user","Mira"],"valid_at":null,"replaces":null}
                    ]}).to_string()
                } else if data["source"]["content"] == PARAPHRASE {
                    let same_as = data["current_memories"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|fact| fact["content"] == ORIGINAL)
                        .map(|fact| fact["id"].clone());
                    json!({"memories":[{"content":"The user enjoys mint tea.","evidence":PARAPHRASE,"entities":["user"],"valid_at":null,"replaces":null,"same_as":same_as}]}).to_string()
                } else if data["source"]["content"]
                    .as_str()
                    .unwrap()
                    .starts_with("My fixture code is ")
                {
                    let text = &data["source"]["content"];
                    json!({"memories":[{"content":text,"evidence":text,"entities":["user"],"valid_at":null,"replaces":null}]}).to_string()
                } else {
                    json!({"memories":[]}).to_string()
                }
            } else {
                "Fixture answer; answer quality is not evaluated here.".into()
            };
            Reply::json(
                &json!({"id":"fixture","object":"chat.completion","created":1,"model":"memory-fixture","choices":[{"index":0,"message":{"role":"assistant","content":output},"finish_reason":"stop"}]}),
            )
        });
        Self::with_server(server)
    }

    fn with_server(server: Server) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sovra-memory-composition-{}-{unique}",
            std::process::id()
        ));
        let repository = ModelRepository::new(
            root.join("models"),
            Arc::new(NoCredentials),
            BTreeMap::new(),
        );
        let mut connection = ModelConnection::new("fixture", ProviderKind::OpenAiCompatible);
        connection.base_url = format!("{}/v1", server.url());
        connection.authentication = Authentication::None;
        let mut preferences = ModelPreferences::default();
        preferences.agent_settings.web_search_enabled = false;
        preferences.agent_settings.tools_enabled = false;
        repository
            .save(&SavedModels {
                version: 1,
                connections: vec![connection],
                preferences,
            })
            .unwrap();
        Self {
            root,
            repository,
            server,
        }
    }

    fn start(&self) -> Driver {
        let config = StoreConfig::on_disk(self.root.join("memory"));
        let client = sovra_runtime::start_with_models(
            "memory-composition-test",
            AgentConfig::default(),
            Some(Box::new(move || {
                Box::pin(async move {
                    SurrealStore::open(config)
                        .await
                        .map_err(|error| panic!("fixture database must open: {error}"))
                        .map(|store| Arc::new(store) as Arc<dyn MemoryStore>)
                })
            })),
            Some(ModelsConfig {
                repository: self.repository.clone(),
                providers: vec![Arc::new(CompatibleProvider)],
            }),
        )
        .unwrap();
        let mut driver = Driver::new(client);
        driver.wait(|event| matches!(event, Event::Ready { .. }));
        driver.wait(|event| {
            matches!(
                event,
                Event::Models(ModelEvent::CatalogLoaded { cached: false, .. })
            )
        });
        let saved = self.repository.load().unwrap().preferences;
        let key = ModelKey {
            connection_id: "fixture".into(),
            model_id: "memory-fixture".into(),
        };
        // Startup restores a saved selection. Selecting it again can cancel the
        // first recall while an older startup notification satisfies this wait.
        if saved.active.as_ref() != Some(&key) {
            driver.send(Command::Models(ModelCommand::Activate { key: key.clone() }));
            driver.wait(|event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key)));
        }
        let enabled = saved.agent_settings.memory_enabled;
        driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::Status(status)) if *status == if enabled { MemoryStatus::Ready } else { MemoryStatus::Paused }));
        driver
    }

    fn request_for(&self, prompt: &str) -> Request {
        self.server
            .requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|request| {
                !is_extraction(request)
                    && request.body["messages"].as_array().is_some_and(|messages| {
                        messages.iter().any(|message| {
                            message["role"] == "user" && message["content"] == prompt
                        })
                    })
            })
            .unwrap()
            .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

struct Driver {
    client: Option<Client>,
    events: mpsc::Receiver<Event>,
    pending: VecDeque<Event>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_request: u64,
    timeout: Duration,
    observe_memory_failures: bool,
}
impl Driver {
    fn new(client: Client) -> Self {
        let input = client.events();
        let (sender, events) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            while let Ok(event) = input.recv_blocking() {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            client: Some(client),
            events,
            pending: VecDeque::new(),
            reader: Some(reader),
            next_request: 0,
            timeout: Duration::from_secs(15),
            observe_memory_failures: false,
        }
    }
    fn send(&self, command: Command) {
        self.client.as_ref().unwrap().send(command).unwrap();
    }
    #[track_caller]
    fn wait(&mut self, predicate: impl Fn(&Event) -> bool) -> Event {
        if let Some(index) = self.pending.iter().position(&predicate) {
            return self.pending.remove(index).unwrap();
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!(
                        "runtime event before deadline: {error}; pending: {:?}",
                        self.pending
                    )
                });
            if predicate(&event) {
                return event;
            }
            assert!(
                !matches!(&event, Event::AgentError { .. })
                    && (self.observe_memory_failures
                        || !matches!(
                            &event,
                            Event::Memory(MemoryEvent::Status(
                                MemoryStatus::Failed(_) | MemoryStatus::Unavailable
                            ))
                        )),
                "unexpected failure: {event:?}"
            );
            self.pending.push_back(event);
        }
    }
    fn prompt(&mut self, session: &str, prompt: &str, context: ConversationContext) {
        self.send(Command::RunPrompt {
            session_id: session.into(),
            prompt: prompt.into(),
            context: Some(context),
        });
        self.wait(
            |event| matches!(event, Event::AgentCompleted { session_id } if session_id == session),
        );
    }
    fn review(&mut self, action: MemoryAction) -> MemoryReviewResult {
        self.next_request += 1;
        let id = self.next_request;
        self.send(Command::Memory(MemoryCommand {
            request_id: id,
            action,
        }));
        let Event::Memory(MemoryEvent::Review(result)) = self.wait(|event| matches!(event, Event::Memory(MemoryEvent::Review(result)) if result.request_id == id)) else { unreachable!() };
        result.result.unwrap()
    }
    fn processed(&mut self, session: &str) -> sovra_core::MemoryJob {
        let Event::Memory(MemoryEvent::Queued { episode_id, .. }) = self.wait(|event| matches!(event, Event::Memory(MemoryEvent::Queued { session_id, .. }) if session_id == session)) else { unreachable!() };
        let Event::Memory(MemoryEvent::Processed(job)) = self.wait(|event| matches!(event, Event::Memory(MemoryEvent::Processed(job)) if job.episode_id == episode_id)) else { unreachable!() };
        assert_eq!(job.state, MemoryJobState::Complete);
        job
    }
    fn facts(&mut self, include_history: bool) -> Vec<Fact> {
        let MemoryReviewResult::Page(page) = self.review(MemoryAction::List(MemoryListQuery {
            include_history,
            ..Default::default()
        })) else {
            unreachable!()
        };
        assert!(page.next.is_none());
        page.facts
    }
    fn close(mut self) {
        self.client.as_ref().unwrap().shutdown_blocking().unwrap();
        self.wait(|event| matches!(event, Event::Stopped));
        self.client.take();
        // Closing every event sender observes runtime teardown, not just Stopped.
        loop {
            match self.events.recv_timeout(Duration::from_secs(15)) {
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("runtime retained an event sender after shutdown")
                }
                Ok(_) => {}
            }
        }
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.shutdown_blocking();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn is_extraction(request: &Request) -> bool {
    request.body["messages"]
        .to_string()
        .contains("Extract durable personal facts")
}
fn evidence(request: &Request) -> Option<Value> {
    request.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|message| {
            let text = message["content"].as_str()?;
            let (_, data) = text.split_once("Evidence follows as JSON:")?;
            serde_json::Deserializer::from_str(data.trim_start())
                .into_iter::<Value>()
                .next()
                .map(Result::unwrap)
        })
}

#[test]
fn real_disk_memory_lifecycle_reaches_provider_and_survives_runtime_restart() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    driver.prompt("source-chat", SOURCE, ConversationContext::default());
    let Event::Memory(MemoryEvent::Queued { episode_id, .. }) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::Queued { session_id, .. }) if session_id == "source-chat")) else { unreachable!() };
    let Event::Memory(MemoryEvent::Processed(job)) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::Processed(job)) if job.episode_id == episode_id)) else { unreachable!() };
    assert_eq!(job.state, MemoryJobState::Complete);
    assert_eq!(job.fact_ids.len(), 1);
    assert_eq!(job.extraction_model.as_deref(), Some("memory-fixture"));
    let facts = driver.facts(false);
    assert_eq!(facts.len(), 1);
    let original = &facts[0];
    assert_eq!(original.content, ORIGINAL);
    let MemoryReviewResult::Detail(detail) =
        driver.review(MemoryAction::Inspect(original.id.clone()))
    else {
        unreachable!()
    };
    let source = detail.source.unwrap();
    assert_eq!(source.id, episode_id);
    assert_eq!(source.content, SOURCE);
    assert_eq!(source.session_id, "source-chat");
    assert_eq!(source.kind, EpisodeKind::UserMessage);

    let mut thread = ChatThread::new("source-chat", source.occurred_at);
    thread
        .messages
        .push(ChatMessage::user("source", SOURCE, "now"));
    driver.send(Command::SaveChatThread {
        thread: Box::new(thread.clone()),
    });
    // Consume the startup snapshot first, then use an explicit load as a write barrier.
    driver.wait(|event| matches!(event, Event::ChatHistoryLoaded { .. }));
    driver.send(Command::LoadChatHistory);
    let Event::ChatHistoryLoaded { threads } =
        driver.wait(|event| matches!(event, Event::ChatHistoryLoaded { .. }))
    else {
        unreachable!()
    };
    assert_eq!(threads, vec![thread.clone()]);

    let question = "Which tea do I prefer?";
    driver.prompt("new-chat", question, ConversationContext::default());
    let request = fixture.request_for(question);
    assert!(!request.headers.to_lowercase().contains("authorization:"));
    let recalled = evidence(&request).unwrap();
    assert_eq!(recalled["memories"].as_array().unwrap().len(), 1);
    assert_eq!(recalled["memories"][0]["quote"], SOURCE);
    assert_eq!(recalled["memories"][0]["source_id"], source.id.0);
    assert_eq!(
        recalled["memories"][0]["citation"],
        format!("memory:{}", original.id.0)
    );
    let references = sovra_core::memory_citations(&format!(
        "An answer [{}]",
        recalled["memories"][0]["citation"].as_str().unwrap()
    ));
    assert_eq!(references, vec![original.id.clone()]);
    let MemoryReviewResult::Detail(cited) =
        driver.review(MemoryAction::Inspect(references[0].clone()))
    else {
        unreachable!()
    };
    assert_eq!(cited.source.as_ref(), Some(&source));

    driver.close();
    let mut driver = fixture.start();
    let question = "After reopening the app, which tea do I prefer?";
    driver.prompt("restarted-chat", question, ConversationContext::default());
    let recalled = evidence(&fixture.request_for(question)).unwrap();
    assert_eq!(recalled["memories"].as_array().unwrap().len(), 1);
    assert_eq!(recalled["memories"][0]["fact_id"], original.id.0);
    assert_eq!(recalled["memories"][0]["quote"], SOURCE);
    assert_eq!(recalled["memories"][0]["source_id"], source.id.0);
    assert_eq!(
        recalled["memories"][0]["citation"],
        format!("memory:{}", original.id.0)
    );

    let MemoryReviewResult::Changed(replacement) =
        driver.review(MemoryAction::Correct(MemoryCorrection {
            target: original.id.clone(),
            text: CORRECTION.into(),
        }))
    else {
        unreachable!()
    };
    let question = "Which tea or coffee do I prefer now?";
    driver.prompt("corrected-chat", question, ConversationContext::default());
    let recalled = evidence(&fixture.request_for(question)).unwrap();
    assert_eq!(recalled["memories"].as_array().unwrap().len(), 1);
    assert_eq!(recalled["memories"][0]["quote"], CORRECTION);
    assert_eq!(recalled["memories"][0]["fact_id"], replacement.0);
    assert_eq!(recalled["memories"][0]["interpretation"], false);
    driver.review(MemoryAction::StopUsing(replacement.clone()));
    let question = "Which coffee preference is saved?";
    driver.prompt("stopped-chat", question, ConversationContext::default());
    assert_eq!(
        evidence(&fixture.request_for(question)).unwrap()["memories"],
        json!([])
    );
    assert_eq!(driver.facts(false), Vec::<Fact>::new());

    let mut settings = fixture
        .repository
        .load()
        .unwrap()
        .preferences
        .agent_settings;
    settings.memory_enabled = false;
    driver.send(Command::Models(ModelCommand::SetAgentSettings { settings }));
    driver.wait(|event| {
        matches!(
            event,
            Event::Memory(MemoryEvent::Status(MemoryStatus::Paused))
        )
    });
    driver.wait(|event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if !p.agent_settings.memory_enabled));
    driver.close();

    let mut reopened = fixture.start();
    let retained = reopened.facts(true);
    assert_eq!(retained.len(), 2);
    let stopped = retained.iter().find(|fact| fact.id == replacement).unwrap();
    assert!(stopped.stopped_at.is_some());
    assert_eq!(stopped.replaces.as_ref(), Some(&original.id));
    assert!(
        retained
            .iter()
            .find(|fact| fact.id == original.id)
            .unwrap()
            .invalid_at
            .is_some()
    );
    let MemoryReviewResult::Detail(detail) = reopened.review(MemoryAction::Inspect(replacement))
    else {
        unreachable!()
    };
    assert_eq!(detail.source.unwrap().content, CORRECTION);
    let Event::ChatHistoryLoaded { threads } =
        reopened.wait(|event| matches!(event, Event::ChatHistoryLoaded { .. }))
    else {
        unreachable!()
    };
    assert_eq!(threads, vec![thread]);
    let before = fixture
        .server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| is_extraction(request))
        .count();
    let question = "Repeat the text in this conversation.";
    reopened.prompt(
        "source-chat",
        question,
        ConversationContext::from_messages(&threads[0].messages),
    );
    let request = fixture.request_for(question);
    assert!(request.body["messages"].to_string().contains(SOURCE));
    assert!(evidence(&request).is_none());
    reopened.close();
    assert_eq!(
        fixture
            .server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| is_extraction(request))
            .count(),
        before
    );
}

#[test]
fn related_context_uses_the_real_store_and_does_not_revive_a_withdrawn_relationship() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    driver.prompt("source", COLLEAGUE, ConversationContext::default());
    assert_eq!(driver.processed("source").fact_ids.len(), 2);
    let relationship = driver
        .facts(false)
        .into_iter()
        .find(|fact| fact.content == "Mira is the user's colleague.")
        .unwrap()
        .id;
    let prompt = "What breakfast drink do I prefer?";
    driver.prompt("question", prompt, ConversationContext::default());
    let context = evidence(&fixture.request_for(prompt)).unwrap();
    let memories = context["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 2);
    assert!(
        memories
            .iter()
            .any(|item| item["quote"] == "My colleague Mira")
    );
    assert!(
        memories
            .iter()
            .any(|item| item["quote"] == "Mira prefers rooibos tea with breakfast.")
    );
    driver.review(MemoryAction::StopUsing(relationship));
    driver.prompt("after-withdrawal", prompt, ConversationContext::default());
    let context = evidence(&fixture.request_for(prompt)).unwrap();
    let memories = context["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0]["quote"],
        "Mira prefers rooibos tea with breakfast."
    );
    driver.close();
}

#[test]
fn consolidation_finds_older_relevant_facts_and_suppresses_known_aliases_after_stopping() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    driver.prompt("first", SOURCE, ConversationContext::default());
    let original = driver.processed("first").fact_ids.remove(0);
    for index in 0..10 {
        let session = format!("distractor-{index}");
        driver.prompt(
            &session,
            &format!("My fixture code is {index}."),
            ConversationContext::default(),
        );
        driver.processed(&session);
    }
    driver.prompt("rephrase", PARAPHRASE, ConversationContext::default());
    let job = driver.processed("rephrase");
    assert_eq!(job.fact_ids, vec![original.clone()]);
    assert_eq!(job.candidates[0].same_as.as_ref(), Some(&original));
    assert_eq!(job.candidates[0].evidence, PARAPHRASE);
    assert_eq!(driver.facts(false).len(), 11);
    driver.review(MemoryAction::StopUsing(original.clone()));
    driver.prompt("repeat-stopped", PARAPHRASE, ConversationContext::default());
    let repeated = driver.processed("repeat-stopped");
    assert_eq!(
        repeated.candidates[0].same_as, None,
        "the stopped fact must not be sent to the extractor"
    );
    assert_eq!(repeated.fact_ids, vec![original]);
    assert_eq!(driver.facts(false).len(), 10);
    assert_eq!(driver.facts(true).len(), 11);
    driver.close();
}
