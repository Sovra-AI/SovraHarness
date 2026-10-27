//! Consent, real HTTP transport and owned cancellation for conversation embeddings.

use super::*;
use sovra_core::{MemoryAction, MemoryCommand, MemoryReviewResult, MemorySemanticPhase};

#[tokio::test]
async fn queued_semantic_disable_keeps_requests_blocked_when_an_earlier_enable_finishes() {
    let server = Server::new(|request| {
        if request.path == "/v1/embeddings" {
            embedding_reply(request)
        } else {
            ordinary_reply(request)
        }
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (release, gate) = async_channel::bounded(2);
    *store.configuration_gate.lock().unwrap() = Some(gate);
    let (fixture, client) = start(&server, store.clone(), true).await;
    configure(&client, 1, Some(target(&fixture)));
    configure(&client, 2, None);
    client.send(Command::Ping).unwrap();
    event(&client, |event| matches!(event, Event::Pong)).await;
    release.send(()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while store.configuration_calls.load(Ordering::SeqCst) != 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let leaked = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.path == "/v1/embeddings");
    release.send(()).await.unwrap();
    configured(&client, 2).await;
    stop(client).await;
    assert!(
        !leaked,
        "the earlier configuration must not undo a later revocation"
    );
}

fn target(fixture: &Fixture) -> ModelDestination {
    let mut saved = fixture.repo.load().unwrap();
    let connection = &mut saved.connections[0];
    let model = ModelInfo {
        id: "memory-embed".into(),
        manual: true,
        output_modalities: vec!["embeddings".into()],
        ..Default::default()
    };
    let target = ModelDestination::for_embeddings(connection, &model).unwrap();
    connection.manual_models.push(model);
    fixture.repo.save(&saved).unwrap();
    target
}

fn configure(client: &Client, id: u64, target: Option<ModelDestination>) {
    client
        .send(Command::Memory(MemoryCommand {
            request_id: id,
            action: MemoryAction::ConfigureSemantic(target),
        }))
        .unwrap();
}

async fn configured(client: &Client, id: u64) {
    event(client, |event| matches!(event, Event::Memory(MemoryEvent::Review(review)) if review.request_id == id && matches!(review.result, Ok(MemoryReviewResult::Semantics(_))))).await;
}

async fn prompt(client: &Client, session: &str, question: &str) {
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: session.into(),
            prompt: question.into(),
        })
        .unwrap();
    event(
        client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == session),
    )
    .await;
}

fn embedding_reply(request: &Request) -> Reply {
    assert_eq!(request.body["model"], "memory-embed");
    let count = request.body["input"].as_array().unwrap().len();
    Reply::json(
        &json!({"data": (0..count).map(|index| json!({"index": index, "embedding": [1.,0.]})).collect::<Vec<_>>()}),
    )
}

fn ordinary_reply(request: &Request) -> Reply {
    if request.path == "/v1/models" {
        Reply::json(&json!({"data":[{"id":"memory-model"}]}))
    } else if is_extraction(request) {
        reply(r#"{"memories":[]}"#)
    } else {
        attachment_reply("memory-model")
    }
}

#[tokio::test]
async fn failed_memory_embedding_batches_wait_for_explicit_retry() {
    let server = Server::new(|request| {
        if request.path == "/v1/embeddings" {
            Reply::json(&json!({"data": []}))
        } else {
            ordinary_reply(request)
        }
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (fixture, client) = start(&server, store.clone(), true).await;
    let target = target(&fixture);
    configure(&client, 1, Some(target.clone()));
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Semantic(status)) if matches!(&status.phase, MemorySemanticPhase::Failed(message) if message.contains("unavailable")))).await;
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == "/v1/embeddings")
            .count(),
        1
    );
    assert_eq!(store.vector_commits.load(Ordering::SeqCst), 0);
    configure(&client, 2, Some(target));
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Semantic(status)) if status.state.generation == 2 && matches!(&status.phase, MemorySemanticPhase::Failed(message) if message.contains("unavailable")))).await;
    assert_eq!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == "/v1/embeddings")
            .count(),
        2
    );
    stop(client).await;
}

#[tokio::test]
async fn slow_semantic_queries_fall_back_to_keyword_evidence_within_the_reader_deadline() {
    let (release, waiting) = std::sync::mpsc::channel();
    let waiting = Arc::new(Mutex::new(waiting));
    let embedding_server = Server::new(move |request| {
        if request.path == "/v1/embeddings" {
            if request.body["input"] == json!(["Which tea?"]) {
                return Reply {
                    status: 200,
                    content_type: "application/json",
                    body: "{\"data\":".into(),
                    tail: Some((
                        waiting.clone(),
                        "[{\"index\":0,\"embedding\":[1,0]}]}".into(),
                    )),
                };
            }
            embedding_reply(request)
        } else {
            ordinary_reply(request)
        }
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    // The fixture serves one socket at a time, so chat uses another endpoint.
    let server = Server::new(ordinary_reply);
    let (fixture, client) = start(&server, store, true).await;
    let mut destination = target(&fixture);
    let mut saved = fixture.repo.load().unwrap();
    let mut connection = saved.connections[0].clone();
    connection.id = "memory-search".into();
    connection.base_url = format!("{}/v1", embedding_server.url());
    destination.key.connection_id.clone_from(&connection.id);
    destination.base_url.clone_from(&connection.base_url);
    saved.connections.push(connection);
    fixture.repo.save(&saved).unwrap();
    configure(&client, 1, Some(destination));
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Semantic(status)) if status.phase == MemorySemanticPhase::Ready && status.state.setting.as_ref().is_some_and(|setting| setting.dimension.is_some()))).await;
    tokio::time::timeout(
        Duration::from_secs(2),
        prompt(&client, "slow", "Which tea?"),
    )
    .await
    .unwrap();
    release.send(()).unwrap();
    assert_eq!(recalled_requests(&server), 1);
    assert!(server.requests.lock().unwrap().iter().any(|r| {
        r.body["messages"]
            .to_string()
            .contains("matching words were still checked")
    }));
    stop(client).await;
}

#[tokio::test]
async fn semantic_memory_requires_separate_consent_and_injects_evidence_without_keyword_overlap() {
    let server = Server::new(|request| {
        if request.path == "/v1/embeddings" {
            embedding_reply(request)
        } else {
            ordinary_reply(request)
        }
    });
    let store = Arc::new(Store::default());
    saved_tea(&store);
    let (fixture, client) = start(&server, store.clone(), true).await;
    let target = target(&fixture);
    prompt(&client, "before", "Suggest beverages").await;
    assert_eq!(recalled_requests(&server), 0);
    assert!(
        !server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.path == "/v1/embeddings")
    );
    configure(&client, 1, Some(target));
    // Ready with a pinned dimension proves the worker committed the HTTP response.
    event(&client, |event| matches!(event, Event::Memory(MemoryEvent::Semantic(status)) if status.phase == MemorySemanticPhase::Ready && status.state.setting.as_ref().is_some_and(|setting| setting.dimension == Some(2)))).await;
    prompt(&client, "after", "Suggest beverages").await;
    assert_eq!(recalled_requests(&server), 1);
    assert_eq!(store.vector_commits.load(Ordering::SeqCst), 1);
    let mut saved = fixture.repo.load().unwrap();
    saved.connections[0].base_url.push_str("/changed");
    fixture.repo.save(&saved).unwrap();
    let count = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path == "/v1/embeddings")
        .count();
    prompt(&client, "changed", "Which tea?").await;
    assert_eq!(
        recalled_requests(&server),
        2,
        "keyword evidence survives semantic failure"
    );
    assert_eq!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == "/v1/embeddings")
            .count(),
        count,
        "changed destinations cannot receive data"
    );
    assert!(server.requests.lock().unwrap().iter().any(|r| {
        r.body["messages"]
            .to_string()
            .contains("matching words were still checked")
    }));
    configure(&client, 2, None);
    configured(&client, 2).await;
    prompt(&client, "disabled", "Suggest beverages").await;
    assert_eq!(recalled_requests(&server), 2);
    stop(client).await;
}

#[tokio::test]
async fn pausing_or_disabling_semantic_memory_during_http_discards_late_vectors() {
    for pause in [true, false] {
        let (release, waiting) = std::sync::mpsc::channel();
        let waiting = Arc::new(Mutex::new(waiting));
        let entered = Arc::new(AtomicBool::new(false));
        let seen = entered.clone();
        let server = Server::new(move |request| {
            if request.path == "/v1/embeddings" {
                seen.store(true, Ordering::SeqCst);
                return Reply {
                    status: 200,
                    content_type: "application/json",
                    body: "{\"data\":".into(),
                    tail: Some((
                        waiting.clone(),
                        "[{\"index\":0,\"embedding\":[1,0]}]}".into(),
                    )),
                };
            }
            ordinary_reply(request)
        });
        let store = Arc::new(Store::default());
        saved_tea(&store);
        let (fixture, client) = start(&server, store.clone(), true).await;
        configure(&client, 1, Some(target(&fixture)));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !entered.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if pause {
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
        } else {
            configure(&client, 2, None);
            configured(&client, 2).await;
        }
        release.send(()).unwrap();
        client.send(Command::Ping).unwrap();
        event(&client, |event| matches!(event, Event::Pong)).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(store.vector_commits.load(Ordering::SeqCst), 0);
        stop(client).await;
    }
}
