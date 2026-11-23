//! Real indexed text crosses the Client, ADK delegation and HTTP provider boundary.

use super::*;
use sovra_core::file_index::{FileIndex, IndexedRoot, ScanCancellation, ScanObserver};

const SOURCE_TEXT: &str = "The orchid greenhouse budget is 420 dollars.";

pub(super) struct Observer;
impl ScanObserver for Observer {
    fn cancellation(&self) -> Option<ScanCancellation> {
        None
    }
    fn progress(&self, _: &IndexedRoot) {}
}

fn reply(request: &Request, mut message: Value, reason: &str) -> Reply {
    if request.body["stream"] == true {
        if let Some(calls) = message["tool_calls"].as_array_mut() {
            for (index, call) in calls.iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
        return Reply {
            status: 200,
            content_type: "text/event-stream",
            tail: None,
            body: format!(
                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                json!({"id":"knowledge-fixture","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":message,"finish_reason":null}]}),
                json!({"id":"knowledge-fixture","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":{},"finish_reason":reason}]})
            ),
        };
    }
    Reply::json(
        &json!({"id":"knowledge-fixture","object":"chat.completion","created":1,"model":"memory-fixture","choices":[{"index":0,"message":message,"finish_reason":reason}]}),
    )
}

fn call(request: &Request, name: &str, args: &Value) -> Reply {
    reply(
        request,
        json!({"role":"assistant","content":null,"tool_calls":[{"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]}),
        "tool_calls",
    )
}

fn model(request: &Request) -> Reply {
    if request.path == "/v1/models" {
        return Reply::json(&json!({"data":[{"id":"memory-fixture"}]}));
    }
    let tools = request.body["tools"].as_array().unwrap();
    let has = |name| tools.iter().any(|tool| tool["function"]["name"] == name);
    let messages = request.body["messages"].as_array().unwrap();
    let result = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .map(|message| {
            serde_json::from_str::<Value>(message["content"].as_str().unwrap()).unwrap()
        });
    let text = request.body.to_string();
    assert!(text.contains("Standing instruction: use plain words."));
    if has("CodingAgent") {
        if let Some(result) = result {
            assert!(result.to_string().contains(SOURCE_TEXT));
            reply(
                request,
                json!({"role":"assistant","content":format!("{SOURCE_TEXT} [budget.txt:1]")}),
                "stop",
            )
        } else {
            call(
                request,
                "CodingAgent",
                &json!({"request":"Read the orchid budget from Knowledge and cite the original passage."}),
            )
        }
    } else {
        assert!(
            has("search_indexed_files"),
            "helper has no index tools: {text}"
        );
        assert!(!has("propose_file_knowledge"));
        if let Some(result) = result {
            if let Some(passages) = result["passages"].as_array() {
                if passages.is_empty() {
                    reply(
                        request,
                        json!({"role":"assistant","content":"No granted source is available."}),
                        "stop",
                    )
                } else {
                    assert_eq!(
                        passages[0]["text"],
                        format!("Source: budget.txt\n\n{SOURCE_TEXT}")
                    );
                    call(
                        request,
                        "read_indexed_file",
                        &json!({"reference":passages[0]["reference"]}),
                    )
                }
            } else {
                assert_eq!(
                    result["passage"]["text"],
                    format!("Source: budget.txt\n\n{SOURCE_TEXT}")
                );
                reply(
                    request,
                    json!({"role":"assistant","content":format!("{SOURCE_TEXT} [budget.txt:1]")}),
                    "stop",
                )
            }
        } else {
            call(
                request,
                "search_indexed_files",
                &json!({"query":"orchid greenhouse budget"}),
            )
        }
    }
}

#[test]
fn indexed_knowledge_reaches_delegates_and_teams_and_revoked_grants_stop_reads() {
    let fixture = Fixture::with_server(Server::new(model));
    let mut saved = fixture.repository.load().unwrap();
    saved.preferences.agent_settings.tools_enabled = true;
    saved.preferences.agent_settings.memory_enabled = false;
    saved.preferences.agent_settings.always_context =
        "Standing instruction: use plain words.".into();
    saved.connections[0].manual_models.push(ModelInfo {
        id: "memory-fixture".into(),
        name: "Knowledge fixture".into(),
        context_length: Some(131_072),
        supported_parameters: vec!["tools".into()],
        ..Default::default()
    });
    fixture.repository.save(&saved).unwrap();
    let source = fixture.root.join("source");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("budget.txt"), SOURCE_TEXT).unwrap();
    let index = Arc::new(sovra_files::LocalFileIndex::new(
        fixture.root.join("file-index"),
    ));
    let root = index.add_root(&source).unwrap();
    index.scan(&root.id, &Observer).unwrap();
    let client = sovra_runtime::start_with_file_index(
        "knowledge-composition",
        AgentConfig::default(),
        None,
        Some(ModelsConfig {
            laya: None,
            readiness: None,
            repository: fixture.repository.clone(),
            providers: vec![Arc::new(CompatibleProvider)],
        }),
        None,
        None,
        Some(index.clone()),
    )
    .unwrap();
    let mut driver = Driver::new(client);
    driver.observe_memory_failures = true;
    driver.wait(|event| {
        matches!(
            event,
            Event::Models(ModelEvent::CatalogLoaded { cached: false, .. })
        )
    });
    let key = ModelKey {
        connection_id: "fixture".into(),
        model_id: "memory-fixture".into(),
    };
    driver.send(Command::Models(ModelCommand::Activate { key: key.clone() }));
    driver.wait(|event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key)));
    driver.prompt(
        "knowledge-chat",
        "Find my orchid budget.",
        ConversationContext::default(),
    );
    assert!(driver.pending.iter().any(|event| matches!(event, Event::AgentChunk { session_id, text } if session_id == "knowledge-chat" && text.contains(SOURCE_TEXT))));
    assert!(
        fixture
            .server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| {
                request.body["messages"].as_array().is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message["role"] == "tool" && message["tool_call_id"] == "call-CodingAgent"
                    })
                })
            }),
        "ordinary chat must actually delegate and consume the helper's reply"
    );
    let member = sovra_core::teams::default_team()
        .into_iter()
        .find(|member| !member.conductor)
        .unwrap();
    let prompt = format!("@{} find my orchid budget.", member.name);
    let run_team = |driver: &mut Driver, session: &str| {
        driver.send(Command::Teams(sovra_core::teams::TeamCommand::Run {
            session_id: session.into(),
            prompt: prompt.clone(),
            context: ConversationContext::default(),
            room: sovra_core::teams::TeamRoom::all_helpers(),
        }));
        let result = driver.wait(|event| matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::RunFinished { .. } | sovra_core::AgUiEvent::RunError { .. } } if session_id == session));
        assert!(
            matches!(
                result,
                Event::AgUi {
                    event: sovra_core::AgUiEvent::RunFinished { .. },
                    ..
                }
            ),
            "{result:?}"
        );
    };
    run_team(&mut driver, "knowledge-team");
    assert!(driver.pending.iter().any(|event| matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::TextMessageContent { delta, .. } } if session_id == "knowledge-team" && delta.contains(SOURCE_TEXT))));
    index.remove(&root.id).unwrap();
    let before = fixture.server.requests.lock().unwrap().len();
    run_team(&mut driver, "revoked-team");
    assert!(driver.pending.iter().any(|event| matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::TextMessageContent { delta, .. } } if session_id == "revoked-team" && delta.contains("No granted source"))));
    assert!(
        fixture.server.requests.lock().unwrap()[before..]
            .iter()
            .all(|request| !request.body.to_string().contains(SOURCE_TEXT))
    );
}

#[test]
fn explorer_combines_three_current_sources_and_respects_edits_and_removal_after_restart() {
    use sovra_core::{
        artifacts::{ArtifactCommand, ArtifactDraft, ArtifactEvent, ArtifactKind, ArtifactSource},
        file_index::{FileIndex, IndexCommand, IndexEvent, IndexStatus},
        knowledge_analysis::{AnalysisOptions, Citation, Exploration, ExploreRequest},
    };
    fn search(driver: &mut Driver, id: u64) -> Exploration {
        driver.send(Command::Explore(ExploreRequest {
            id,
            query: "Juniper".into(),
            options: AnalysisOptions::default(),
        }));
        let Event::Explored { result, .. } =
            driver.wait(|e| matches!(e,Event::Explored{request_id,..} if *request_id==id))
        else {
            unreachable!()
        };
        result.unwrap()
    }
    let fixture = Fixture::new();
    let folder = fixture.root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("sample.txt"), "Juniper sample weighs 0.5 kg.").unwrap();
    let store = Arc::new(sovra_files::LocalFileIndex::new(fixture.root.join("index")));
    let root = store.add_root(&folder).unwrap();
    store.set_auto_refresh(&root.id, false).unwrap();
    let mut driver = fixture.start_with_index(Some(store.clone()));
    driver.send(Command::FileIndex(IndexCommand::Refresh {
        root_id: root.id.clone(),
    }));
    driver.wait(|e|matches!(e,Event::FileIndex(IndexEvent::Progress(r)) if r.status==IndexStatus::Ready && r.progress.indexed==1));
    let MemoryReviewResult::Changed(note) =
        driver.review(MemoryAction::Remember(sovra_core::MemoryNote {
            text: "My Juniper reading was 500 g.".into(),
        }))
    else {
        unreachable!()
    };
    driver.send(Command::Artifacts(ArtifactCommand::Save(ArtifactDraft {
        title: "Juniper calibration".into(),
        kind: ArtifactKind::Table,
        language: "csv".into(),
        source: ArtifactSource::default(),
        content: "timestamp,value,unit,instrument\n2026-06-01,500,g,Juniper".into(),
    })));
    let Event::Artifacts(ArtifactEvent::Saved(item)) =
        driver.wait(|e| matches!(e, Event::Artifacts(ArtifactEvent::Saved(_))))
    else {
        unreachable!()
    };
    driver.close();
    let mut driver = fixture.start_with_index(Some(store));
    let found = search(&mut driver, 1);
    assert!(
        found
            .readings
            .iter()
            .any(|r| matches!(r.excerpt.citation, Citation::File { .. })),
        "{found:?}"
    );
    assert!(
        found
            .readings
            .iter()
            .any(|r| matches!(r.excerpt.citation, Citation::User { .. })),
        "{found:?}"
    );
    assert!(
        found
            .readings
            .iter()
            .any(|r| matches!(r.excerpt.citation, Citation::Artifact { .. })),
        "{found:?}"
    );
    assert!(
        found.shared.iter().any(|s| s.label.contains("500 g")
            && s.sources.len() == 3
            && s.sources.iter().any(|line| found.readings[line.reading]
                .excerpt
                .title
                .contains("sample.txt"))
            && s.sources
                .iter()
                .any(
                    |line| found.readings[line.reading].excerpt.title == "Juniper calibration"
                        && line.line == 2
                )),
        "{found:?}"
    );
    driver.send(Command::Artifacts(ArtifactCommand::Edit {
        id: item.id.clone(),
        title: "Updated calibration".into(),
        content: "Oak sample".into(),
    }));
    driver.wait(
        |e| matches!(e,Event::Artifacts(ArtifactEvent::Saved(a)) if a.title=="Updated calibration"),
    );
    driver.review(MemoryAction::Remove(note));
    driver.send(Command::FileIndex(IndexCommand::Remove {
        root_id: root.id,
    }));
    driver.wait(|e| matches!(e,Event::FileIndex(IndexEvent::Roots(roots)) if roots.is_empty()));
    assert_eq!(search(&mut driver, 2).readings, Vec::new());
    driver.close();
}
