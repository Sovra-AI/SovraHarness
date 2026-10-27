//! Runtime protocol tests with a deterministic HTTP model and observable memory port.

use super::*;
use sovra_core::{
    Episode, EpisodeId, FactId, MemoryCandidate, MemoryClaim, MemoryError, MemoryEvent,
    MemoryInput, MemoryJob, MemoryJobState, MemoryPolicy, MemoryStatus, MemoryStore, NewEpisode,
    NewFact, RecallQuery, RecalledFact, Timestamp,
};

#[derive(Default)]
struct Store {
    chats: Mutex<BTreeMap<String, sovra_core::ChatThread>>,
    chat_started: Option<async_channel::Sender<String>>,
    chat_gate: Option<async_channel::Receiver<()>>,
    facts: Mutex<Vec<sovra_core::Fact>>,
    semantic: Mutex<sovra_core::MemorySemanticState>,
    vectors: Mutex<Vec<sovra_core::MemoryVector>>,
    vector_commits: AtomicU64,
    configuration_gate: Mutex<Option<async_channel::Receiver<()>>>,
    configuration_calls: AtomicU64,
    sources: Mutex<Vec<Episode>>,
    recalls: Mutex<Vec<RecallQuery>>,
    recall_gate: Mutex<Option<async_channel::Receiver<()>>>,
    related_gate: Mutex<Option<async_channel::Receiver<()>>>,
    related_started: Option<async_channel::Sender<()>>,
    pending: Mutex<Option<MemoryInput>>,
    inputs: Mutex<Vec<MemoryInput>>,
    completed: Mutex<Vec<Vec<MemoryCandidate>>>,
    failures: Mutex<Vec<String>>,
    review_actions: Mutex<Vec<sovra_core::MemoryAction>>,
    generation: AtomicU64,
    enabled: AtomicBool,
}
impl Store {
    fn source(input: &MemoryInput) -> Episode {
        Episode {
            id: EpisodeId("episode:test".into()),
            session_id: input.episode.session_id.clone(),
            kind: input.episode.kind,
            content: input.episode.content.clone(),
            occurred_at: input.episode.occurred_at,
        }
    }
}
#[async_trait::async_trait]
impl MemoryStore for Store {
    async fn save_chat_thread(&self, thread: &sovra_core::ChatThread) -> Result<(), MemoryError> {
        if let Some(started) = &self.chat_started {
            let _ = started.send(thread.title.clone()).await;
        }
        if thread.title == "held" {
            self.chat_gate.as_ref().unwrap().recv().await.unwrap();
        }
        self.chats
            .lock()
            .unwrap()
            .insert(thread.id.clone(), thread.clone());
        Ok(())
    }
    async fn list_chat_threads(
        &self,
        _: usize,
    ) -> Result<Vec<sovra_core::ChatThread>, MemoryError> {
        Ok(self
            .chats
            .lock()
            .unwrap()
            .values()
            .filter(|thread| thread.deleted_at.is_none())
            .cloned()
            .collect())
    }
    async fn delete_chat_thread(&self, id: &str, at: Timestamp) -> Result<(), MemoryError> {
        if let Some(thread) = self.chats.lock().unwrap().get_mut(id) {
            thread.deleted_at = Some(at);
        }
        Ok(())
    }
    async fn memory_semantics(&self) -> Result<sovra_core::MemorySemanticState, MemoryError> {
        Ok(self.semantic.lock().unwrap().clone())
    }
    async fn configure_memory_semantics(
        &self,
        target: Option<ModelDestination>,
    ) -> Result<sovra_core::MemorySemanticState, MemoryError> {
        self.configuration_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self.configuration_gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            let _ = gate.recv().await;
        }
        let mut state = self.semantic.lock().unwrap();
        state.generation += 1;
        state.setting = target.map(|target| sovra_core::MemorySemanticSetting {
            generation: state.generation,
            target,
            dimension: None,
        });
        self.vectors.lock().unwrap().clear();
        Ok(state.clone())
    }
    async fn memory_embedding_inputs(
        &self,
        _: &sovra_core::MemorySemanticSetting,
        ids: &[FactId],
    ) -> Result<Vec<sovra_core::MemoryEmbeddingInput>, MemoryError> {
        let vectors = self.vectors.lock().unwrap();
        Ok(self
            .facts
            .lock()
            .unwrap()
            .iter()
            .filter(|fact| {
                ids.contains(&fact.id) && !vectors.iter().any(|vector| vector.fact_id == fact.id)
            })
            .map(|fact| sovra_core::MemoryEmbeddingInput {
                fact_id: fact.id.clone(),
                text: fact.content.clone(),
            })
            .collect())
    }
    async fn commit_memory_vectors(
        &self,
        _: &sovra_core::MemorySemanticSetting,
        _: MemoryPolicy,
        vectors: Vec<sovra_core::MemoryVector>,
    ) -> Result<(), MemoryError> {
        // Deliberately permissive: cancellation must prevent this call, even without adapter fences.
        self.vector_commits.fetch_add(1, Ordering::SeqCst);
        if let Some(setting) = &mut self.semantic.lock().unwrap().setting {
            setting.dimension = vectors.first().map(|item| item.vector.values().len());
        }
        self.vectors.lock().unwrap().extend(vectors);
        Ok(())
    }
    async fn semantic_memories(
        &self,
        _: &sovra_core::MemorySemanticSetting,
        _: &sovra_core::file_index::semantic::EmbeddingVector,
        _: sovra_core::MemorySearch,
        _: Timestamp,
    ) -> Result<Vec<sovra_core::MemorySemanticHit>, MemoryError> {
        Ok(self
            .facts
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .map(|fact| sovra_core::MemorySemanticHit {
                fact,
                similarity: 1.,
            })
            .collect())
    }
    async fn health(&self) -> Result<(), MemoryError> {
        Ok(())
    }
    async fn memory_policy(&self) -> Result<MemoryPolicy, MemoryError> {
        Ok(MemoryPolicy {
            enabled: self.enabled.load(Ordering::SeqCst),
            generation: self.generation.load(Ordering::SeqCst),
        })
    }
    async fn set_memory_enabled(&self, enabled: bool) -> Result<MemoryPolicy, MemoryError> {
        if self.enabled.swap(enabled, Ordering::SeqCst) != enabled {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
        if !enabled {
            *self.pending.lock().unwrap() = None;
        }
        self.memory_policy().await
    }
    async fn enqueue_memory(&self, input: MemoryInput) -> Result<EpisodeId, MemoryError> {
        self.inputs.lock().unwrap().push(input.clone());
        *self.pending.lock().unwrap() = Some(input);
        Ok(EpisodeId("episode:test".into()))
    }
    async fn claim_memory(&self, at: Timestamp) -> Result<Option<MemoryClaim>, MemoryError> {
        Ok(self
            .pending
            .lock()
            .unwrap()
            .take()
            .map(|input| MemoryClaim {
                episode: Self::source(&input),
                token: "lease".into(),
                generation: self.generation.load(Ordering::SeqCst),
                attempt: 1,
                lease_until: Timestamp(at.millis() + 120_000),
            }))
    }
    async fn complete_memory(
        &self,
        _claim: &MemoryClaim,
        candidates: Vec<MemoryCandidate>,
        _model: &str,
        _at: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        // Deliberately accepts any call: the test detects whether runtime cancellation
        // prevents a late commit, without relying on an adapter's extra protection.
        self.completed.lock().unwrap().push(candidates);
        Ok(vec![FactId("fact:test".into())])
    }
    async fn fail_memory(
        &self,
        _claim: &MemoryClaim,
        message: &str,
        _at: Timestamp,
    ) -> Result<(), MemoryError> {
        self.failures.lock().unwrap().push(message.into());
        Ok(())
    }
    async fn memory_job(&self, id: &EpisodeId) -> Result<MemoryJob, MemoryError> {
        Ok(MemoryJob {
            episode_id: id.clone(),
            state: if self.failures.lock().unwrap().is_empty() {
                MemoryJobState::Complete
            } else {
                MemoryJobState::Failed
            },
            attempts: 1,
            error: self.failures.lock().unwrap().last().cloned(),
            fact_ids: vec![],
            candidates: vec![],
            extraction_model: None,
        })
    }
    async fn record_episode(&self, _episode: NewEpisode) -> Result<EpisodeId, MemoryError> {
        Ok(EpisodeId("episode:reply".into()))
    }
    async fn episode(&self, id: &EpisodeId) -> Result<Episode, MemoryError> {
        self.sources
            .lock()
            .unwrap()
            .iter()
            .find(|source| &source.id == id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound("unused".into()))
    }
    async fn recent_episodes(
        &self,
        _session: &str,
        _limit: usize,
    ) -> Result<Vec<Episode>, MemoryError> {
        Ok(Vec::new())
    }
    async fn save_fact(&self, _fact: NewFact) -> Result<FactId, MemoryError> {
        Err(MemoryError::InvalidInput(
            "runtime must use atomic completion".into(),
        ))
    }
    async fn invalidate_fact(&self, _: &FactId, _: Timestamp) -> Result<(), MemoryError> {
        panic!("explicit user stops must use the withdrawal port");
    }
    async fn stop_memory(&self, id: &FactId, _at: Timestamp) -> Result<(), MemoryError> {
        self.review_actions
            .lock()
            .unwrap()
            .push(sovra_core::MemoryAction::StopUsing(id.clone()));
        Ok(())
    }
    async fn list_memories(
        &self,
        query: sovra_core::MemoryListQuery,
    ) -> Result<sovra_core::MemoryPage, MemoryError> {
        self.review_actions
            .lock()
            .unwrap()
            .push(sovra_core::MemoryAction::List(query));
        Ok(sovra_core::MemoryPage {
            facts: self.facts.lock().unwrap().clone(),
            next: None,
        })
    }
    async fn memory_fact(&self, id: &FactId) -> Result<sovra_core::Fact, MemoryError> {
        self.review_actions
            .lock()
            .unwrap()
            .push(sovra_core::MemoryAction::Inspect(id.clone()));
        self.facts
            .lock()
            .unwrap()
            .iter()
            .find(|fact| &fact.id == id)
            .cloned()
            .ok_or_else(|| MemoryError::NotFound(id.0.clone()))
    }
    async fn correct_memory(
        &self,
        correction: sovra_core::MemoryCorrection,
    ) -> Result<FactId, MemoryError> {
        self.review_actions
            .lock()
            .unwrap()
            .push(sovra_core::MemoryAction::Correct(correction));
        Ok(FactId("fact:corrected".into()))
    }
    async fn recall(&self, query: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        self.recalls.lock().unwrap().push(query.clone());
        if query.text.is_none() && query.entity.is_some() {
            if let Some(started) = &self.related_started {
                let _ = started.send(()).await;
            }
            let gate = self.related_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                let _ = gate.recv().await;
            }
        }
        let gate = self.recall_gate.lock().unwrap().clone();
        if query.text.is_some()
            && let Some(gate) = gate
        {
            let _ = gate.recv().await;
        }
        Ok(self
            .facts
            .lock()
            .unwrap()
            .iter()
            .filter(|fact| {
                query
                    .text
                    .as_ref()
                    .is_none_or(|text| fact.content.to_lowercase().contains(&text.to_lowercase()))
            })
            .map(|fact| RecalledFact {
                fact: fact.clone(),
                score: 1.,
            })
            .collect())
    }
    async fn load_team(&self) -> Result<Option<Vec<sovra_core::teams::TeamMember>>, MemoryError> {
        Ok(Some(
            sovra_core::teams::default_team()
                .into_iter()
                .filter(|member| !member.conductor)
                .take(1)
                .collect(),
        ))
    }
}

fn reply(text: &str) -> Reply {
    Reply::json(
        &json!({"id":"m","object":"chat.completion","created":1,"model":"memory-model","choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}]}),
    )
}
fn valid() -> &'static str {
    r#"{"memories":[{"content":"User lives in Lisbon","evidence":"I live in Lisbon","entities":["Lisbon"],"valid_at":null,"replaces":null}]}"#
}
fn is_extraction(request: &Request) -> bool {
    request.body["messages"]
        .to_string()
        .contains("Extract durable personal facts")
}
async fn start(server: &Server, store: Arc<Store>, enabled: bool) -> (Fixture, Client) {
    let mut connection = ModelConnection::new("custom", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.authentication = Authentication::None;
    let fixture = Fixture::new(connection, None);
    let mut saved = fixture.repo.load().unwrap();
    saved.preferences.agent_settings.memory_enabled = enabled;
    fixture.repo.save(&saved).unwrap();
    let client = sovra_runtime::start_with_files(
        "test",
        AgentConfig::default(),
        Some(Box::new(move || {
            Box::pin(async move { Ok(store as Arc<dyn MemoryStore>) })
        })),
        Some(ModelsConfig {
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(CompatibleProvider)],
        }),
        Some(Arc::new(AttachmentFixture)),
    )
    .unwrap();
    catalog(&client).await;
    send(
        &client,
        ModelCommand::Activate {
            key: ModelKey {
                connection_id: "custom".into(),
                model_id: "memory-model".into(),
            },
        },
    );
    event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.is_some())).await;
    (fixture, client)
}

#[tokio::test]
async fn selected_model_remembers_only_user_evidence_without_blocking_chat_or_ping() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model"}]}));
        }
        if is_extraction(request) {
            assert_eq!(request.body["model"], "memory-model");
            assert!(
                request
                    .body
                    .get("tools")
                    .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
            );
            return reply(valid());
        }
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s1".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    event(&client, |event| {
        matches!(event, Event::Memory(MemoryEvent::Processed(_)))
    })
    .await;
    assert_eq!(store.inputs.lock().unwrap().len(), 1);
    assert_eq!(
        store.completed.lock().unwrap()[0][0].content,
        "User lives in Lisbon"
    );
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    stop(client).await;
}

#[tokio::test]
async fn pausing_during_extraction_cancels_late_results_and_new_inputs() {
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = Arc::new(Mutex::new(waiting));
    let entered = Arc::new(AtomicBool::new(false));
    let seen = entered.clone();
    let server = Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model"}]}));
        }
        if is_extraction(request) {
            seen.store(true, Ordering::SeqCst);
            let _ = waiting.lock().unwrap().recv_timeout(Duration::from_secs(5));
            return reply(valid());
        }
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s1".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    send(
        &client,
        ModelCommand::SetAgentSettings {
            settings: AgentSettings {
                memory_enabled: false,
                ..Default::default()
            },
        },
    );
    event(&client, |event| {
        matches!(
            event,
            Event::Memory(MemoryEvent::Status(MemoryStatus::Paused))
        )
    })
    .await;
    release.send(()).unwrap();
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s2".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(store.completed.lock().unwrap().len(), 0);
    assert_eq!(store.inputs.lock().unwrap().len(), 1);
    stop(client).await;
}

#[tokio::test]
async fn saved_disabled_setting_prevents_startup_work() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model"}]}));
        }
        assert!(!is_extraction(request));
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    *store.pending.lock().unwrap() = Some(MemoryInput {
        key: "old".into(),
        episode: NewEpisode {
            session_id: "old".into(),
            kind: sovra_core::EpisodeKind::UserMessage,
            content: "I live in Lisbon".into(),
            occurred_at: Timestamp(1),
        },
    });
    let (_fixture, client) = start(&server, store.clone(), false).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s1".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    event(&client, |event| {
        matches!(event, Event::AgentCompleted { .. })
    })
    .await;
    assert_eq!(store.inputs.lock().unwrap().len(), 0);
    assert_eq!(store.completed.lock().unwrap().len(), 0);
    stop(client).await;
}

#[tokio::test]
async fn unsupported_extraction_is_visible_and_never_committed() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model"}]}));
        }
        if is_extraction(request) {
            return reply(&valid().replace("I live in Lisbon", "I live in Rome"));
        }
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s1".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    event(&client, |event| {
        matches!(
            event,
            Event::Memory(MemoryEvent::Status(MemoryStatus::Failed(_)))
        )
    })
    .await;
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Processed(job)) if job.state == MemoryJobState::Failed)).await;
    assert_eq!(store.completed.lock().unwrap().len(), 0);
    assert_eq!(store.failures.lock().unwrap().len(), 1);
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    stop(client).await;
}

#[tokio::test]
async fn changing_the_selected_model_cancels_its_in_flight_extraction() {
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = Arc::new(Mutex::new(waiting));
    let entered = Arc::new(AtomicBool::new(false));
    let seen = entered.clone();
    let server = Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"memory-model"},{"id":"replacement-model"}]}),
            );
        }
        if is_extraction(request) {
            seen.store(true, Ordering::SeqCst);
            let _ = waiting.lock().unwrap().recv_timeout(Duration::from_secs(5));
            return reply(valid());
        }
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "s1".into(),
            prompt: "I live in Lisbon".into(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    send(
        &client,
        ModelCommand::Activate {
            key: ModelKey {
                connection_id: "custom".into(),
                model_id: "replacement-model".into(),
            },
        },
    );
    event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref().is_some_and(|key| key.model_id == "replacement-model"))).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.failures.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(store.completed.lock().unwrap().len(), 0);
    assert!(store.failures.lock().unwrap()[0].contains("selected AI changed"));
    stop(client).await;
}

fn saved_tea(store: &Store) {
    let source = Episode {
        id: EpisodeId("episode:earlier".into()),
        session_id: "previous-conversation".into(),
        kind: sovra_core::EpisodeKind::UserMessage,
        content: "I prefer jasmine tea".into(),
        occurred_at: Timestamp(100),
    };
    store.facts.lock().unwrap().push(sovra_core::Fact {
        id: FactId("fact:tea".into()),
        content: "User prefers jasmine tea".into(),
        entities: vec![],
        source: Some(source.id.clone()),
        replaces: None,
        extraction: Some(sovra_core::MemoryExtraction {
            model: "previous-model".into(),
            evidence: source.content.clone(),
        }),
        created_at: Timestamp(200),
        valid_at: Some(Timestamp(100)),
        invalid_at: None,
        expired_at: None,
        stopped_at: None,
    });
    store.sources.lock().unwrap().push(source);
}
fn recall_server() -> Server {
    Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model"}]}));
        }
        if is_extraction(request) {
            return reply(r#"{"memories":[]}"#);
        }
        attachment_reply("memory-model")
    })
}

#[tokio::test]
async fn memory_review_uses_the_storage_port_while_paused_and_drains_accepted_edits() {
    use sovra_core::{
        MemoryAction, MemoryCommand, MemoryCorrection, MemoryListQuery, MemoryReviewResult,
    };
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (_fixture, client) = start(&server, store.clone(), false).await;
    for (request_id, action) in [
        (
            1,
            MemoryAction::List(MemoryListQuery {
                search: "tea".into(),
                include_history: true,
                ..Default::default()
            }),
        ),
        (2, MemoryAction::Inspect(FactId("fact:tea".into()))),
        (
            3,
            MemoryAction::Correct(MemoryCorrection {
                target: FactId("fact:tea".into()),
                text: "I prefer mint tea".into(),
            }),
        ),
    ] {
        client
            .send(Command::Memory(MemoryCommand { request_id, action }))
            .unwrap();
        let response = event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Review(event)) if event.request_id == request_id)).await;
        let Event::Memory(MemoryEvent::Review(response)) = response else {
            unreachable!()
        };
        match response.result.unwrap() {
            MemoryReviewResult::Semantics(_) => panic!("no semantic setting was requested"),
            MemoryReviewResult::Page(page) => assert_eq!(page.facts[0].id.0, "fact:tea"),
            MemoryReviewResult::Detail(detail) => {
                assert_eq!(detail.source.unwrap().content, "I prefer jasmine tea");
            }
            MemoryReviewResult::Changed(id) => assert_eq!(id.0, "fact:corrected"),
        }
    }
    client
        .send(Command::Memory(MemoryCommand {
            request_id: 4,
            action: MemoryAction::StopUsing(FactId("fact:tea".into())),
        }))
        .unwrap();
    stop(client).await;
    let actions = store.review_actions.lock().unwrap();
    assert_eq!(actions.len(), 4);
    assert!(matches!(&actions[3], MemoryAction::StopUsing(id) if id.0 == "fact:tea"));
    assert!(!store.enabled.load(Ordering::SeqCst));
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.path.ends_with("completions"))
    );
}
fn recalled_requests(server: &Server) -> usize {
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            !is_extraction(request)
                && request.body["messages"]
                    .to_string()
                    .contains("memory:fact:tea")
        })
        .count()
}

#[tokio::test]
async fn ordinary_and_team_turns_can_plan_historical_memory_queries_over_real_http() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"memory-model", "supported_parameters":["tools"]}]}),
            );
        }
        if is_extraction(request) {
            return reply(r#"{"memories":[]}"#);
        }
        assert!(
            request.body["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["function"]["name"] == "search_saved_details")
        );
        let tool_result = request.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "tool");
        let (message, finish) = if let Some(receipt) = tool_result {
            assert!(
                !receipt.to_string().contains("jasmine"),
                "tool receipts must not persist remembered facts"
            );
            assert!(
                request.body["messages"]
                    .to_string()
                    .contains("memory:fact:tea")
            );
            (
                json!({"role":"assistant", "content":"You preferred jasmine tea then [memory:fact:tea]."}),
                "stop",
            )
        } else {
            assert!(
                !request.body["messages"]
                    .to_string()
                    .contains("memory:fact:tea"),
                "current recall must exclude this old version"
            );
            let args = json!({"query":"tea", "entity":"User", "valid_at":"2020-06-01", "known_at":"2020-12-01"}).to_string();
            (
                json!({"role":"assistant", "tool_calls":[{"index":0,"id":"lookup-history","type":"function","function":{"name":"search_saved_details", "arguments":args}}]}),
                "tool_calls",
            )
        };
        Reply {
            status: 200,
            content_type: "text/event-stream",
            tail: None,
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"history","object":"chat.completion.chunk","created":1,"model":"memory-model","choices":[{"index":0,"delta":message,"finish_reason":null}]}),
                json!({"id":"history","object":"chat.completion.chunk","created":1,"model":"memory-model","choices":[{"index":0,"delta":{},"finish_reason":finish}]})
            ),
        }
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    {
        let mut facts = store.facts.lock().unwrap();
        let fact = &mut facts[0];
        fact.entities = vec!["User".into()];
        fact.valid_at = Some(Timestamp(1_577_836_800_000));
        fact.invalid_at = Some(Timestamp(1_609_459_200_000));
        fact.expired_at = Some(Timestamp(1_640_995_200_000));
    }
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "past-chat".into(),
            prompt: "What tea did I prefer on June 1, 2020, as known on December 1, 2020?".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "past-chat"),
    )
    .await;
    client
        .send(Command::Teams(sovra_core::teams::TeamCommand::Run {
            session_id: "past-team".into(),
            prompt: "What tea did I prefer on June 1, 2020, as known on December 1, 2020?".into(),
            history: vec![],
        }))
        .unwrap();
    event(&client, |event| matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::RunFinished { .. } } if session_id == "past-team")).await;
    assert_eq!(recalled_requests(&server), 2);
    assert!(
        store
            .recalls
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.valid_at == Some(Timestamp(1_590_969_600_000))
                && query.known_at == Some(Timestamp(1_606_780_800_000))
                && query.entity.as_deref() == Some("User")
                && query.include_expired)
    );
    stop(client).await;
}

#[tokio::test]
async fn selected_model_budget_trims_restored_context_and_rejects_oversized_current_questions() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-model", "context_length":8192}]}));
        }
        if is_extraction(request) {
            return reply(r#"{"memories":[]}"#);
        }
        let text = request.body.to_string();
        assert!(text.contains("CURRENT tea question"));
        assert!(!text.contains("OLDEST"));
        assert!(text.contains("shortened to fit"));
        assert!(!text.contains("sovra_optional_context"));
        assert_eq!(request.body["max_completion_tokens"], 2048);
        assert!(
            text.len() + 2048 < 8192,
            "assembled HTTP request plus reply reserve must fit this fixture"
        );
        attachment_reply("memory-model")
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (_fixture, client) = start(&server, store, true).await;
    let messages: Vec<_> = (0..24)
        .map(|i| {
            sovra_core::ChatMessage::user(
                format!("m-{i}"),
                format!(
                    "{} {}",
                    if i == 0 { "OLDEST" } else { "earlier" },
                    "context ".repeat(100)
                ),
                "",
            )
        })
        .collect();
    let context = sovra_core::ConversationContext::from_messages(&messages);
    client
        .send(Command::RunPrompt {
            context: Some(context),
            session_id: "budgeted".into(),
            prompt: "CURRENT tea question".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "budgeted"),
    )
    .await;
    client
        .send(Command::RunPrompt {
            context: Some(sovra_core::ConversationContext::default()),
            session_id: "oversized".into(),
            prompt: "x".repeat(9000),
        })
        .unwrap();
    event(&client, |event| matches!(event, Event::AgentError { session_id, message } if session_id == "oversized" && message.contains("input budget"))).await;
    assert_eq!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.path == "/v1/chat/completions" && !is_extraction(request))
            .count(),
        1
    );
    assert_eq!(
        messages.len(),
        24,
        "request trimming never changes saved conversation data"
    );
    stop(client).await;
}

#[tokio::test]
async fn ordinary_and_team_turns_receive_cited_cross_conversation_evidence() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "new-chat".into(),
            prompt: "Which tea do I prefer?".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "new-chat"),
    )
    .await;
    assert_eq!(recalled_requests(&server), 1);
    let ordinary = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| {
            !is_extraction(request)
                && request.body["messages"]
                    .to_string()
                    .contains("memory:fact:tea")
        })
        .unwrap()
        .body
        .clone();
    let text = ordinary.to_string();
    assert!(text.contains("previous-conversation"));
    assert!(text.contains("I prefer jasmine tea"));
    assert!(text.contains("untrusted data"));
    client
        .send(Command::Teams(sovra_core::teams::TeamCommand::Run {
            session_id: "new-team".into(),
            prompt: "Recommend tea for me".into(),
            history: vec![],
        }))
        .unwrap();
    event(&client, |event| matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::RunFinished { .. } } if session_id == "new-team")).await;
    assert!(recalled_requests(&server) >= 2);
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| !is_extraction(request))
            .any(|request| request.body["messages"]
                .to_string()
                .contains("Recommend tea for me")),
        "a team request must include the current prompt even when history is empty"
    );
    send(
        &client,
        ModelCommand::SetAgentSettings {
            settings: AgentSettings {
                memory_enabled: false,
                ..Default::default()
            },
        },
    );
    event(&client, |event| {
        matches!(
            event,
            Event::Memory(MemoryEvent::Status(MemoryStatus::Paused))
        )
    })
    .await;
    let before = recalled_requests(&server);
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "new-chat".into(),
            prompt: "Tea again?".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "new-chat"),
    )
    .await;
    assert_eq!(
        recalled_requests(&server),
        before,
        "injected evidence must not enter provider session history"
    );
    stop(client).await;
}

#[tokio::test]
async fn pausing_during_recall_discards_results_without_blocking_commands() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (release, gate) = async_channel::bounded(1);
    *store.recall_gate.lock().unwrap() = Some(gate);
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "new-chat".into(),
            prompt: "Which tea?".into(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !store
            .recalls
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.text.is_some())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    send(
        &client,
        ModelCommand::SetAgentSettings {
            settings: AgentSettings {
                memory_enabled: false,
                ..Default::default()
            },
        },
    );
    event(&client, |event| {
        matches!(
            event,
            Event::Memory(MemoryEvent::Status(MemoryStatus::Paused))
        )
    })
    .await;
    let _ = release.try_send(());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path.ends_with("completions") && !is_extraction(request))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(recalled_requests(&server), 0);
    stop(client).await;
}

#[tokio::test]
async fn stopping_a_memory_revokes_recall_already_in_flight() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (release, gate) = async_channel::bounded(1);
    *store.recall_gate.lock().unwrap() = Some(gate);
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "held-recall".into(),
            prompt: "Which tea?".into(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !store
            .recalls
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.text.is_some())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    client
        .send(Command::Memory(sovra_core::MemoryCommand {
            request_id: 1,
            action: sovra_core::MemoryAction::StopUsing(FactId("fact:tea".into())),
        }))
        .unwrap();
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Review(event)) if event.request_id == 1 && event.result.is_ok())).await;
    // This port deliberately still returns the old snapshot, as a read started
    // before the stop can do. The runtime must revoke it before provider delivery.
    let _ = release.try_send(());
    tokio::time::timeout(Duration::from_secs(5), async {
        while !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path.ends_with("completions") && !is_extraction(request))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(recalled_requests(&server), 0);
    stop(client).await;
}

#[tokio::test]
async fn unrelated_expired_future_and_assistant_evidence_never_enters_requests() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let original = store.facts.lock().unwrap()[0].clone();
    {
        let mut facts = store.facts.lock().unwrap();
        facts[0].invalid_at = Some(Timestamp(300));
        facts[0].expired_at = Some(Timestamp(300));
        let mut future = original.clone();
        future.id.0 = "fact:future".into();
        future.valid_at = Some(Timestamp(i64::MAX));
        facts.push(future);
        let mut assistant = original.clone();
        assistant.id.0 = "fact:assistant".into();
        assistant.source = Some(EpisodeId("episode:assistant".into()));
        facts.push(assistant);
        let mut forged = original;
        forged.id.0 = "fact:forged".into();
        forged.extraction.as_mut().unwrap().evidence = "I prefer coffee".into();
        facts.push(forged);
    }
    store.sources.lock().unwrap().push(Episode {
        id: EpisodeId("episode:assistant".into()),
        session_id: "old".into(),
        kind: sovra_core::EpisodeKind::AgentMessage,
        content: "I prefer jasmine tea".into(),
        occurred_at: Timestamp(100),
    });
    let (_fixture, client) = start(&server, store.clone(), true).await;
    for (id, prompt) in [
        ("unrelated", "Explain black holes"),
        ("unsupported", "Which tea?"),
    ] {
        client
            .send(Command::RunPrompt {
                context: None,
                session_id: id.into(),
                prompt: prompt.into(),
            })
            .unwrap();
        event(
            &client,
            |event| matches!(event, Event::AgentCompleted { session_id } if session_id == id),
        )
        .await;
    }
    for request in server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| !is_extraction(request))
    {
        let messages = request.body["messages"].to_string();
        assert!(!messages.contains("memory:fact:"));
        assert!(!messages.contains("I prefer jasmine tea"));
    }
    stop(client).await;
}

#[tokio::test]
async fn attachment_turns_recall_user_memory_without_learning_file_contents() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunWithFiles {
            context: None,
            session_id: "files".into(),
            prompt: "Read this invoice and recommend tea".into(),
            attachment_ids: vec!["test-document".into()],
        })
        .unwrap();
    // Remembering has its own worker and may enqueue after the answer completes.
    let answered = std::cell::Cell::new(false);
    let queued = std::cell::Cell::new(false);
    event(&client, |event| {
        if matches!(event, Event::AgentCompleted { session_id } if session_id == "files") {
            answered.set(true);
        }
        if matches!(event, Event::Memory(MemoryEvent::Queued { session_id, .. }) if session_id == "files") {
            queued.set(true);
        }
        answered.get() && queued.get()
    })
    .await;
    assert_eq!(recalled_requests(&server), 1);
    let request = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| !is_extraction(request) && request.path.ends_with("completions"))
        .unwrap()
        .body
        .clone();
    assert!(request.to_string().contains("93 dirhams"));
    let inputs = store.inputs.lock().unwrap().clone();
    assert_eq!(inputs.len(), 1);
    assert_eq!(
        inputs[0].episode.content,
        "Read this invoice and recommend tea"
    );
    stop(client).await;
}

#[tokio::test]
async fn visible_conversation_context_survives_restart_and_model_change_with_remembering_off() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"memory-model"},{"id":"replacement-model"}]}),
            );
        }
        assert!(!is_extraction(request));
        attachment_reply(request.body["model"].as_str().unwrap())
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (fixture, client) = start(&server, store, false).await;
    stop(client).await;
    let restored_store = Arc::new(Store::default());
    saved_tea(&restored_store);
    let client = sovra_runtime::start_with_models(
        "test",
        AgentConfig::default(),
        Some(Box::new(move || {
            Box::pin(async move { Ok(restored_store as Arc<dyn MemoryStore>) })
        })),
        Some(ModelsConfig {
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(CompatibleProvider)],
        }),
    )
    .unwrap();
    catalog(&client).await;
    let history = vec![
        sovra_core::ChatMessage::user("u", "Our garden app uses Rust", ""),
        sovra_core::ChatMessage::assistant("a", "We chose an offline design", "", vec![]),
    ];
    for (model, prompt) in [
        ("memory-model", "What language did we choose?"),
        ("replacement-model", "What design did we choose?"),
    ] {
        send(
            &client,
            ModelCommand::Activate {
                key: ModelKey {
                    connection_id: "custom".into(),
                    model_id: model.into(),
                },
            },
        );
        event(&client, |event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref().is_some_and(|key| key.model_id == model))).await;
        client
            .send(Command::RunPrompt {
                session_id: "restored-chat".into(),
                prompt: prompt.into(),
                context: Some(sovra_core::ConversationContext::from_messages(&history)),
            })
            .unwrap();
        event(&client, |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "restored-chat")).await;
    }
    let requests = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.path.ends_with("completions"))
        .map(|request| request.body.clone())
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let data = request.to_string();
        assert_eq!(data.matches("Our garden app uses Rust").count(), 1);
        assert_eq!(data.matches("We chose an offline design").count(), 1);
        assert!(!data.contains("memory:fact:tea"));
        let messages = request["messages"].as_array().unwrap();
        let earlier = messages
            .iter()
            .find(|message| message["content"].as_str() == Some("We chose an offline design"))
            .unwrap();
        assert_eq!(earlier["role"], "assistant");
    }
    stop(client).await;
}

#[path = "memory/semantic.rs"]
mod semantic;

#[path = "memory/history.rs"]
mod history;

#[path = "memory/related.rs"]
mod related;
