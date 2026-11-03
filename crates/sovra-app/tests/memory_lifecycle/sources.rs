//! Source-only recall and bounded overlapping work through Client and real disk storage.

use super::*;
use std::sync::Mutex;

#[test]
fn reformulated_lookup_batch_recovers_old_details_beyond_the_working_set_after_restart() {
    let server = Server::new(|request| {
        if request.path == "/v1/models" {
            return Reply::json(
                &json!({"data":[{"id":"memory-fixture","supported_parameters":["tools"]}]}),
            );
        }
        let (message, finish) = if is_extraction(request) {
            (
                json!({"role":"assistant","content":"{\"memories\":[]}"}),
                "stop",
            )
        } else if request.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool")
        {
            (
                json!({"role":"assistant","content":"Fixture answer; this checks retrieval, not answer quality."}),
                "stop",
            )
        } else {
            let calls = ["monthly income", "salary arrives"]
                .into_iter()
                .enumerate()
                .map(|(index, query)| {
                    json!({"index":index,"id":format!("recall-{index}"),"type":"function","function":{
                        "name":"search_saved_details","arguments":json!({"query":query}).to_string()
                    }})
                })
                .collect::<Vec<_>>();
            (json!({"role":"assistant","tool_calls":calls}), "tool_calls")
        };
        if request.body["stream"] == true {
            Reply {
                status: 200,
                content_type: "text/event-stream",
                tail: None,
                body: format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"id":"old-memory","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":message,"finish_reason":null}]}),
                    json!({"id":"old-memory","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":{},"finish_reason":finish}]})
                ),
            }
        } else {
            Reply::json(
                &json!({"id":"old-memory","object":"chat.completion","created":1,"model":"memory-fixture","choices":[{"index":0,"message":message,"finish_reason":finish}]}),
            )
        }
    });
    let fixture = Fixture::with_server(server);
    let mut saved = fixture.repository.load().unwrap();
    saved.preferences.agent_settings.tools_enabled = true;
    fixture.repository.save(&saved).unwrap();
    let mut driver = fixture.start();
    let income = "My monthly income is 9,200 credits.";
    let payday = "My salary arrives on the 25th.";
    let mut saved_ids = Vec::new();
    for text in [income, payday].into_iter().map(str::to_owned).chain(
        (0..32).map(|index| format!("My synthetic garden plot number {index} grows lavender.")),
    ) {
        let MemoryReviewResult::Changed(id) =
            driver.review(MemoryAction::Remember(sovra_core::MemoryNote { text }))
        else {
            panic!("saved note")
        };
        saved_ids.push(id);
    }
    driver.close();
    let mut driver = fixture.start();
    let prompt = "How much do I earn, and when is it paid?";
    driver.prompt("older-details", prompt, ConversationContext::default());
    {
        let requests = fixture.server.requests.lock().unwrap();
        let first = requests
            .iter()
            .find(|request| !is_extraction(request) && request.path.ends_with("completions"))
            .unwrap();
        let evidence = first.body["messages"].to_string();
        assert!(!evidence.contains(income));
        assert!(!evidence.contains(payday));
    }
    let recalled = fixture.request_for(prompt).body["messages"].to_string();
    assert!(recalled.contains(income), "{recalled}");
    assert!(recalled.contains(payday));
    assert!(
        !recalled.contains("synthetic garden plot"),
        "focused lookups must omit unrelated recent background details"
    );
    assert!(recalled.contains(&format!("memory:{}", saved_ids[0].0)));
    assert!(recalled.contains(&format!("memory:{}", saved_ids[1].0)));
    driver.review(MemoryAction::Remove(saved_ids[0].clone()));
    driver.close();
    let mut driver = fixture.start();
    driver.prompt(
        "older-details-after-removal",
        prompt,
        ConversationContext::default(),
    );
    let recalled = fixture.request_for(prompt).body["messages"].to_string();
    assert!(!recalled.contains(income));
    assert!(recalled.contains(payday));
    driver.close();
}

#[test]
fn saved_income_is_available_without_keyword_overlap_or_embedding_service() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    let statement = "My monthly income is 9,200 credits.";
    let MemoryReviewResult::Changed(id) =
        driver.review(MemoryAction::Remember(sovra_core::MemoryNote {
            text: statement.into(),
        }))
    else {
        panic!("saved note")
    };
    driver.close();
    let mut driver = fixture.start();
    let prompt = "how much do I earn?";
    driver.prompt("paraphrased-income", prompt, ConversationContext::default());
    let request = fixture.request_for(prompt);
    let evidence = request.body["messages"].to_string();
    assert!(
        evidence.contains(statement),
        "a saved personal detail must not depend on exact question wording: {evidence}"
    );
    assert!(evidence.contains(&format!("memory:{}", id.0)));
    let changed = "My monthly income is now 10,500 credits.";
    let MemoryReviewResult::Changed(replacement) =
        driver.review(MemoryAction::Correct(MemoryCorrection {
            target: id,
            text: changed.into(),
        }))
    else {
        panic!("corrected note")
    };
    driver.prompt("updated-income", prompt, ConversationContext::default());
    let evidence = fixture.request_for(prompt).body["messages"].to_string();
    assert!(evidence.contains(changed));
    assert!(
        !evidence.contains(statement),
        "current background must not retain the old detail"
    );
    driver.review(MemoryAction::Remove(replacement));
    driver.close();
    let mut driver = fixture.start();
    driver.prompt("removed-income", prompt, ConversationContext::default());
    let evidence = fixture.request_for(prompt).body["messages"].to_string();
    assert!(!evidence.contains(changed));
    assert!(!evidence.contains(statement));
    // Exact source words must not recover the old original note after removal.
    let historical_prompt = "What was my monthly income? Use saved evidence.";
    driver.prompt(
        "removed-income-history",
        historical_prompt,
        ConversationContext::default(),
    );
    let evidence = fixture.request_for(historical_prompt).body["messages"].to_string();
    assert!(!evidence.contains(changed));
    assert!(!evidence.contains(statement));
    assert!(
        fixture
            .repository
            .load()
            .unwrap()
            .preferences
            .agent_settings
            .memory_enabled
    );
    driver.close();
}

#[test]
fn reused_memory_lists_each_supporting_record_after_reopening() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    driver.prompt("support-first", SOURCE, ConversationContext::default());
    let first = driver.processed("support-first");
    driver.prompt("support-repeat", PARAPHRASE, ConversationContext::default());
    let repeated = driver.processed("support-repeat");
    assert_eq!(first.fact_ids, repeated.fact_ids);
    driver.prompt(
        "support-unrelated",
        COLLEAGUE,
        ConversationContext::default(),
    );
    driver.processed("support-unrelated");
    driver.close();
    let mut driver = fixture.start();
    let mut query = sovra_core::MemorySourceQuery {
        fact: Some(first.fact_ids[0].clone()),
        limit: 1,
        ..Default::default()
    };
    let mut found = vec![];
    for _ in 0..3 {
        let MemoryReviewResult::Sources(page) =
            driver.review(MemoryAction::ListSources(query.clone()))
        else {
            panic!("supporting sources")
        };
        assert_eq!(page.sources.len(), 1);
        let id = page.sources[0].id.clone();
        let MemoryReviewResult::Source(record) =
            driver.review(MemoryAction::InspectSource(id.clone()))
        else {
            panic!("original wording")
        };
        assert!([SOURCE, PARAPHRASE].contains(&record.episode.content.as_str()));
        found.push(id);
        query.before = page.next;
        if query.before.is_none() {
            break;
        }
    }
    assert!(query.before.is_none());
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = vec![first.episode_id, repeated.episode_id];
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(found, expected);
    driver.close();
}

#[test]
fn reviewing_a_detail_exposes_its_real_source_connections_through_runtime_events() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    driver.prompt("explore-mira", COLLEAGUE, ConversationContext::default());
    let job = driver.processed("explore-mira");
    assert_eq!(job.fact_ids.len(), 2);
    let MemoryReviewResult::Detail(detail) =
        driver.review(MemoryAction::Inspect(job.fact_ids[0].clone()))
    else {
        panic!("detail review result")
    };
    assert!(
        detail
            .neighborhood
            .connections
            .iter()
            .any(|link| link.fact.id == job.fact_ids[1]
                && link.reason == sovra_core::MemoryConnectionReason::SameSource)
    );
    assert_eq!(detail.source.unwrap().content, COLLEAGUE);
    driver.close();
    let mut driver = fixture.start();
    let MemoryReviewResult::Subjects(subjects) = driver.review(MemoryAction::ListSubjects(
        sovra_core::MemorySubjectQuery::default(),
    )) else {
        panic!("subject directory after reopening")
    };
    assert_eq!(
        subjects.subjects,
        vec![sovra_core::MemorySubject {
            name: "Mira".into(),
            details: 2
        }]
    );
    driver.close();
}

#[test]
fn omitted_evidence_reaches_the_provider_after_reopening_without_a_fact() {
    let fixture = Fixture::new();
    let mut driver = fixture.start();
    let source = "My rehearsal code is cedarlock.";
    driver.prompt("omitted-evidence", source, ConversationContext::default());
    let job = driver.processed("omitted-evidence");
    assert_eq!(job.fact_ids, Vec::new());
    driver.close();
    let mut driver = fixture.start();
    let MemoryReviewResult::Sources(page) =
        driver.review(MemoryAction::ListSources(sovra_core::MemorySourceQuery {
            search: "cedarlock".into(),
            ..Default::default()
        }))
    else {
        panic!("source library after reopening")
    };
    assert_eq!(page.sources.len(), 1);
    assert_eq!(page.sources[0].id, job.episode_id);
    assert_eq!(page.sources[0].metadata.detail_count, Some(0));
    assert_eq!(
        page.sources[0].metadata.processing,
        Some(MemoryJobState::Complete)
    );
    let MemoryReviewResult::Source(record) =
        driver.review(MemoryAction::InspectSource(job.episode_id.clone()))
    else {
        panic!("original evidence")
    };
    assert_eq!(record.episode.content, source);
    assert_eq!(
        record.metadata.origin,
        sovra_core::MemorySourceOrigin::UserStatement
    );
    driver.prompt(
        "recall-evidence",
        "What rehearsal code did I give you?",
        ConversationContext::default(),
    );
    let requests = fixture.server.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        !is_extraction(request)
            && request
                .body
                .to_string()
                .contains(&format!("source:{}", job.episode_id.0))
            && request.body.to_string().contains(source)
    }));
    drop(requests);
    driver.close();
}

#[test]
fn four_extractions_hold_leases_while_the_fifth_waits_and_ping_remains_live() {
    let (release, gate) = mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let held = std::sync::atomic::AtomicBool::new(false);
    let fixture = Fixture::with_server(Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-fixture"}]}));
        }
        let extraction = is_extraction(request);
        let mut reply = Reply::json(
            &json!({"id":"fixture","object":"chat.completion","created":1,
            "model":"memory-fixture","choices":[{"index":0,"message":{"role":"assistant",
            "content":if extraction { "{\"memories\":[]}" } else { "Fixture answer." }},"finish_reason":"stop"}]}),
        );
        if extraction && !held.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let body = std::mem::take(&mut reply.body);
            reply.tail = Some((gate.clone(), body));
        }
        reply
    }));
    let mut driver = fixture.start();
    for index in 0..5 {
        driver.send(Command::RunPrompt {
            session_id: format!("overlap-{index}"),
            prompt: format!("My rehearsal code is cedar{index}."),
            context: None,
        });
    }
    for _ in 0..4 {
        driver.wait(|event| {
            matches!(
                event,
                Event::Memory(MemoryEvent::Status(MemoryStatus::Working))
            )
        });
    }
    driver.send(Command::Ping);
    driver.wait(|event| matches!(event, Event::Pong));
    let MemoryReviewResult::Page(page) =
        driver.review(MemoryAction::List(MemoryListQuery::default()))
    else {
        unreachable!()
    };
    let overview = page.overview.unwrap();
    assert_eq!(overview.working, 4);
    assert_eq!(overview.waiting, 1);
    release.send(()).unwrap();
    for index in 0..5 {
        driver.processed(&format!("overlap-{index}"));
    }
    driver.close();
}

fn assistant_fixture() -> Fixture {
    assistant_fixture_with_gate(None)
}

fn assistant_fixture_with_gate(gate: Option<Arc<Mutex<mpsc::Receiver<()>>>>) -> Fixture {
    Fixture::with_server(Server::new(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"memory-fixture"}]}));
        }
        if is_extraction(request) {
            return Reply::json(
                &json!({"id":"fixture","object":"chat.completion","created":1,"model":"memory-fixture","choices":[{"index":0,"message":{"role":"assistant","content":"{\"memories\":[]}"},"finish_reason":"stop"}]}),
            );
        }
        let mut body = format!(
            "data: {}\n\n",
            json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":{"role":"assistant","content":"Fixture answer."},"finish_reason":null}]})
        );
        let end = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"memory-fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        );
        let tail = gate.as_ref().map(|gate| (gate.clone(), end.clone()));
        if tail.is_none() {
            body.push_str(&end);
        }
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body,
            tail,
        }
    }))
}

#[test]
fn assistant_evidence_keeps_explicit_authorship_after_reopening() {
    let fixture = assistant_fixture();
    let mut driver = fixture.start();
    driver.prompt(
        "attributed-answer",
        "Hello there.",
        ConversationContext::default(),
    );
    let Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) = driver.wait(|event|
        matches!(event, Event::Memory(MemoryEvent::SourceSaved { session_id, receipt, .. }) if session_id == "attributed-answer" && !receipt.has_processing_job))
        else { panic!("durable evidence receipt") };
    assert!(!receipt.has_processing_job);
    driver.processed("attributed-answer");
    driver.close();
    let mut driver = fixture.start();
    let MemoryReviewResult::Sources(page) =
        driver.review(MemoryAction::ListSources(sovra_core::MemorySourceQuery {
            search: "Fixture answer".into(),
            ..Default::default()
        }))
    else {
        panic!("saved assistant reply")
    };
    assert_eq!(page.sources.len(), 1);
    let source = &page.sources[0];
    assert_eq!(source.id, receipt.source_id);
    assert_eq!(
        source.metadata.origin,
        sovra_core::MemorySourceOrigin::AssistantOutput
    );
    assert_eq!(
        source.metadata.scope,
        sovra_core::MemorySourceScope::Private
    );
    assert_eq!(source.metadata.processing, None);
    assert!(source.metadata.admitted_at.is_some());
    let attribution = source.metadata.attribution.as_ref().unwrap();
    assert_eq!(
        attribution.author,
        sovra_core::MemorySourceAuthor::Assistant { speaker: None }
    );
    assert_eq!(
        attribution.provenance,
        sovra_core::MemorySourceProvenance::Conversation
    );
    driver.close();
}

#[test]
fn team_reply_evidence_keeps_member_and_run_identity_across_restart() {
    let fixture = assistant_fixture();
    let mut driver = fixture.start();
    let mut run_ids = Vec::new();
    let mut source_ids = Vec::new();
    for _ in 0..2 {
        driver.send(Command::Teams(sovra_core::teams::TeamCommand::Run {
            session_id: "attributed-team".into(),
            prompt: "@priya Say a fictional greeting.".into(),
            context: ConversationContext::default(),
        }));
        let Event::AgUi {
            event: sovra_core::AgUiEvent::RunFinished { run_id },
            ..
        } = driver.wait(|event| {
            matches!(
                event,
                Event::AgUi {
                    event: sovra_core::AgUiEvent::RunFinished { .. },
                    ..
                }
            )
        })
        else {
            unreachable!()
        };
        let Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) = driver.wait(|event|
            matches!(event, Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) if !receipt.has_processing_job))
        else { unreachable!() };
        run_ids.push(run_id);
        source_ids.push(receipt.source_id);
        driver.processed("attributed-team");
    }
    assert_ne!(run_ids[0], run_ids[1]);
    assert_ne!(source_ids[0], source_ids[1]);
    driver.close();
    let mut reopened = fixture.start();
    let query = sovra_core::MemorySourceQuery {
        search: "Fixture answer".into(),
        ..Default::default()
    };
    let MemoryReviewResult::Sources(page) =
        reopened.review(MemoryAction::ListSources(query.clone()))
    else {
        unreachable!()
    };
    assert_eq!(
        page.sources.len(),
        2,
        "each visible team reply must remain reachable as original evidence"
    );
    for source in &page.sources {
        let index = source_ids.iter().position(|id| id == &source.id).unwrap();
        assert_eq!(
            source.metadata.origin,
            sovra_core::MemorySourceOrigin::AssistantOutput
        );
        assert_eq!(source.metadata.processing, None);
        let attribution = source.metadata.attribution.as_ref().unwrap();
        assert_eq!(
            attribution.author,
            sovra_core::MemorySourceAuthor::Assistant {
                speaker: Some("Priya".into())
            }
        );
        assert_eq!(
            attribution.provenance,
            sovra_core::MemorySourceProvenance::Team {
                run_id: run_ids[index].clone(),
                member_id: "priya".into()
            }
        );
    }
    assert_eq!(
        reopened.facts(false),
        Vec::new(),
        "a helper's statement is not a personal fact"
    );
    reopened.review(MemoryAction::StopSource(source_ids[0].clone()));
    reopened.close();
    let mut reopened = fixture.start();
    let MemoryReviewResult::Sources(page) = reopened.review(MemoryAction::ListSources(query))
    else {
        unreachable!()
    };
    assert_eq!(page.sources.len(), 1);
    assert_eq!(page.sources[0].id, source_ids[1]);
    let MemoryReviewResult::Source(stopped) =
        reopened.review(MemoryAction::InspectSource(source_ids[0].clone()))
    else {
        unreachable!()
    };
    assert!(stopped.metadata.revoked_at.is_some());
    assert_eq!(stopped.episode.content, "Fixture answer.");
    reopened.close();
}

#[test]
fn pausing_then_resuming_during_a_reply_does_not_revive_its_capture() {
    for team in [false, true] {
        let (release, held) = mpsc::channel();
        let fixture = assistant_fixture_with_gate(Some(Arc::new(Mutex::new(held))));
        let mut driver = fixture.start();
        let session = "capture-across-pause";
        driver.send(if team {
            Command::Teams(sovra_core::teams::TeamCommand::Run {
                session_id: session.into(),
                prompt: "@priya Say a fictional greeting.".into(),
                context: ConversationContext::default(),
            })
        } else {
            Command::RunPrompt {
                session_id: session.into(),
                prompt: "Say a fictional greeting.".into(),
                context: None,
            }
        });
        driver.wait(|event| matches!(event,
            Event::AgentChunk { text, .. } if text.contains("Fixture answer"))
            || matches!(event, Event::AgUi { event: sovra_core::AgUiEvent::TextMessageContent { delta, .. }, .. } if delta.contains("Fixture answer")));
        for enabled in [false, true] {
            let mut settings = fixture
                .repository
                .load()
                .unwrap()
                .preferences
                .agent_settings;
            settings.memory_enabled = enabled;
            driver.send(Command::Models(ModelCommand::SetAgentSettings { settings }));
            driver.wait(|event| matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.agent_settings.memory_enabled == enabled));
        }
        release.send(()).unwrap();
        driver.wait(|event| {
            matches!(event, Event::AgentCompleted { .. })
                || matches!(
                    event,
                    Event::AgUi {
                        event: sovra_core::AgUiEvent::RunFinished { .. },
                        ..
                    }
                )
        });
        driver.close();
        let mut reopened = fixture.start();
        let MemoryReviewResult::Sources(page) =
            reopened.review(MemoryAction::ListSources(sovra_core::MemorySourceQuery {
                search: "Fixture answer".into(),
                ..Default::default()
            }))
        else {
            unreachable!()
        };
        assert_eq!(
            page.sources,
            Vec::new(),
            "a pause invalidates the whole in-flight reply, team={team}"
        );
        reopened.close();
    }
}

#[test]
fn replies_completed_in_other_conversations_survive_real_runtime_and_disk_reopen() {
    let fixture = assistant_fixture();
    for (namespace, team) in [(71, false), (72, true)] {
        let mut driver = fixture.start();
        let mut workspace =
            sovra_core::WorkspaceState::new_at(namespace, sovra_core::Timestamp(100));
        let original = workspace.session_id.clone();
        workspace.team_mode = team;
        let prompt = if team {
            "@priya Give a fictional greeting."
        } else {
            "Give a fictional greeting."
        };
        workspace.submit_prompt(prompt);
        driver.send(workspace.prompt_command(prompt));
        let visible = workspace.start_new_chat().id;
        workspace.submit_prompt("Keep this unrelated draft private to its own conversation.");
        let completed = driver.wait(|event| matches!(event, Event::AgentCompleted { session_id } if session_id == &original)
            || matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::RunFinished { .. } } if session_id == &original));
        workspace.observe_time(sovra_core::Timestamp(200));
        for event in &driver.pending {
            workspace.apply_event(event);
        }
        workspace.apply_event(&completed);
        assert_eq!(workspace.session_id, visible);
        assert_eq!(workspace.messages.len(), 1);
        let snapshot = workspace
            .threads
            .iter()
            .find(|thread| thread.id == original)
            .unwrap()
            .clone();
        assert!(
            snapshot
                .messages
                .iter()
                .any(|message| message.text == "Fixture answer.")
        );
        driver.send(Command::SaveChatThread {
            thread: Box::new(snapshot.clone()),
        });
        driver.processed(&original);
        driver.close();
        let mut reopened = fixture.start();
        let Event::ChatHistoryLoaded { threads } =
            reopened.wait(|event| matches!(event, Event::ChatHistoryLoaded { .. }))
        else {
            unreachable!()
        };
        assert_eq!(
            threads.iter().find(|thread| thread.id == original),
            Some(&snapshot)
        );
        assert!(
            snapshot
                .messages
                .iter()
                .all(|message| !message.text.contains("unrelated draft"))
        );
        reopened.close();
    }
}

fn visible_turn(driver: &mut Driver, workspace: &mut sovra_core::WorkspaceState, prompt: &str) {
    let session = workspace.session_id.clone();
    workspace.submit_prompt(prompt);
    driver.send(workspace.prompt_command(prompt));
    let completed = driver.wait(|event| matches!(event, Event::AgentCompleted { session_id } if session_id == &session)
        || matches!(event, Event::AgUi { session_id, event: sovra_core::AgUiEvent::RunFinished { .. } } if session_id == &session));
    driver.processed(&session);
    for event in std::mem::take(&mut driver.pending) {
        if event.session_id() == Some(session.as_str()) {
            workspace.apply_event(&event);
        }
    }
    workspace.apply_event(&completed);
    driver.send(Command::SaveChatThread {
        thread: Box::new(workspace.active_thread().unwrap().clone()),
    });
}

#[test]
fn linked_helper_recall_survives_restart_and_ancestor_withdrawal_removes_only_its_descendants() {
    for team in [false, true] {
        let fixture = assistant_fixture();
        let mut driver = fixture.start();
        let mut workspace = sovra_core::WorkspaceState::new_at(89, sovra_core::Timestamp(100));
        workspace.team_mode = team;
        let prompt = if team {
            "@priya Say hello."
        } else {
            "Say hello."
        };
        visible_turn(&mut driver, &mut workspace, prompt);
        let first = workspace
            .messages
            .iter()
            .find(|m| m.text == "Fixture answer.")
            .unwrap()
            .source
            .clone()
            .expect("linked first reply");
        assert!(
            workspace.messages[0].source.is_some(),
            "user receipt links the input"
        );
        visible_turn(
            &mut driver,
            &mut workspace,
            if team {
                "@priya Continue the suggestion."
            } else {
                "Continue the suggestion."
            },
        );
        let second = workspace
            .messages
            .iter()
            .rev()
            .find(|m| m.text == "Fixture answer.")
            .unwrap()
            .source
            .clone()
            .expect("linked second reply");
        assert_ne!(first, second);
        let snapshot = workspace.active_thread().unwrap().clone();
        driver.close();
        let mut driver = fixture.start();
        let Event::ChatHistoryLoaded { threads } =
            driver.wait(|event| matches!(event, Event::ChatHistoryLoaded { .. }))
        else {
            unreachable!()
        };
        assert_eq!(
            threads.iter().find(|thread| thread.id == snapshot.id),
            Some(&snapshot)
        );
        driver.prompt(
            "independent",
            "Independent greeting.",
            ConversationContext::default(),
        );
        let Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::SourceSaved { session_id, receipt, .. }) if session_id == "independent" && !receipt.has_processing_job)) else { unreachable!() };
        let independent = receipt.source_id;
        driver.prompt(
            "lookup-before",
            "Find the Fixture answer",
            ConversationContext::default(),
        );
        let request = fixture
            .request_for("Find the Fixture answer")
            .body
            .to_string();
        for id in [&first, &second, &independent] {
            assert!(
                request.contains(&format!("source:{}", id.0)),
                "helper evidence missing (team={team}): {id:?}; {request}"
            );
        }
        assert!(request.contains("assistant_output"));
        driver.review(MemoryAction::StopSource(first.clone()));
        driver.prompt(
            "lookup-after",
            "Find the Fixture answer after withdrawal",
            ConversationContext::default(),
        );
        let request = fixture
            .request_for("Find the Fixture answer after withdrawal")
            .body
            .to_string();
        for id in [&first, &second] {
            assert!(
                !request.contains(&format!("source:{}", id.0)),
                "withdrawn helper ancestry returned"
            );
        }
        assert!(request.contains(&format!("source:{}", independent.0)));
        driver.close();
    }
}

#[test]
fn helper_recall_tracks_automatic_saved_facts_and_excludes_unknown_visible_history() {
    let fixture = assistant_fixture();
    let mut driver = fixture.start();
    let MemoryReviewResult::Changed(fact) =
        driver.review(MemoryAction::Remember(sovra_core::MemoryNote {
            text: "The fixture preference is mint tea.".into(),
        }))
    else {
        unreachable!()
    };
    driver.prompt(
        "fact-dependent",
        "Suggest something",
        ConversationContext::default(),
    );
    let Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::SourceSaved { session_id, receipt, .. }) if session_id == "fact-dependent" && !receipt.has_processing_job)) else { unreachable!() };
    let dependent = receipt.source_id;
    driver.prompt(
        "unknown-history",
        "Continue",
        ConversationContext::from_messages(&[ChatMessage::user(
            "old",
            "Untracked prior wording",
            "",
        )]),
    );
    let Event::Memory(MemoryEvent::SourceSaved { receipt, .. }) = driver.wait(|event| matches!(event, Event::Memory(MemoryEvent::SourceSaved { session_id, receipt, .. }) if session_id == "unknown-history" && !receipt.has_processing_job)) else { unreachable!() };
    let unknown = receipt.source_id;
    driver.prompt(
        "before-removal",
        "Find Fixture answer",
        ConversationContext::default(),
    );
    let request = fixture.request_for("Find Fixture answer").body.to_string();
    assert!(request.contains(&format!("source:{}", dependent.0)));
    assert!(!request.contains(&format!("source:{}", unknown.0)));
    driver.review(MemoryAction::Remove(fact));
    driver.prompt(
        "after-removal",
        "Find Fixture answer now",
        ConversationContext::default(),
    );
    let request = fixture
        .request_for("Find Fixture answer now")
        .body
        .to_string();
    assert!(!request.contains(&format!("source:{}", dependent.0)));
    assert!(!request.contains(&format!("source:{}", unknown.0)));
    driver.close();
}
