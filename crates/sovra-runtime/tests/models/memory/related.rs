//! A retrieved preference needs bounded context identifying its subject.

use super::*;

fn memory_request(server: &Server) -> String {
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| request.path == "/v1/chat/completions" && !is_extraction(request))
        .unwrap()
        .body["messages"]
        .to_string()
}

pub(super) fn evidence_sections(request: &Value) -> Value {
    let evidence = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["content"].as_str())
        .find_map(|text| {
            text.split_once("Evidence follows as JSON:\n")
                .map(|(_, data)| data)
        })
        .expect("provider received structured memory evidence");
    serde_json::from_str(evidence).unwrap()
}

fn first_evidence(server: &Server) -> Value {
    let requests = server.requests.lock().unwrap();
    let request = requests
        .iter()
        .find(|request| request.path == "/v1/chat/completions" && !is_extraction(request))
        .unwrap();
    evidence_sections(&request.body)
}

fn colleague(store: &Store) {
    saved_tea(store);
    let text = "My colleague Mira prefers rooibos tea with breakfast.";
    store.sources.lock().unwrap()[0].content = text.into();
    let mut facts = store.facts.lock().unwrap();
    facts[0].content = "Mira prefers rooibos tea with breakfast.".into();
    facts[0].entities = vec!["Mira".into()];
    facts[0].extraction.as_mut().unwrap().evidence =
        "Mira prefers rooibos tea with breakfast.".into();
    let mut relationship = facts[0].clone();
    relationship.id.0 = "fact:colleague".into();
    relationship.content = "Mira is the user's colleague.".into();
    relationship.entities = vec!["user".into(), "Mira".into()];
    relationship.extraction.as_mut().unwrap().evidence = "My colleague Mira".into();
    facts.push(relationship);
}

#[tokio::test]
async fn recall_adds_related_identity_without_unrelated_or_withdrawn_facts() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    colleague(&store);
    {
        let mut facts = store.facts.lock().unwrap();
        let relationship = facts[1].clone();
        for id in ["unrelated", "withdrawn", "future", "expired", "forged"] {
            let mut fact = relationship.clone();
            fact.id.0 = format!("fact:{id}");
            match id {
                "unrelated" => fact.entities = vec!["user".into(), "Nora".into()],
                "withdrawn" => fact.stopped_at = Some(Timestamp(300)),
                "future" => fact.valid_at = Some(Timestamp(i64::MAX)),
                "expired" => {
                    fact.invalid_at = Some(Timestamp(300));
                    fact.expired_at = Some(Timestamp(400));
                }
                "forged" => {
                    fact.extraction.as_mut().unwrap().evidence = "Unsupported relationship".into();
                }
                _ => unreachable!(),
            }
            facts.push(fact);
        }
    }
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: Some(sovra_core::ConversationContext::default()),
            session_id: "related".into(),
            prompt: "What breakfast drink do I prefer?".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "related"),
    )
    .await;
    {
        let requests = server.requests.lock().unwrap();
        let request = requests
            .iter()
            .find(|request| request.path == "/v1/chat/completions" && !is_extraction(request))
            .unwrap();
        let messages = request.body["messages"].to_string();
        assert!(messages.contains("memory:fact:tea"));
        assert!(messages.contains("memory:fact:colleague"));
        assert!(messages.contains("My colleague Mira"));
        for id in ["withdrawn", "future", "expired", "forged"] {
            assert!(!messages.contains(&format!("memory:fact:{id}")));
        }
        let evidence = evidence_sections(&request.body);
        assert!(
            !evidence["memories"]
                .to_string()
                .contains("memory:fact:unrelated")
        );
        assert!(
            evidence["background"]
                .to_string()
                .contains("memory:fact:unrelated")
        );
    }
    let related = store
        .recalls
        .lock()
        .unwrap()
        .iter()
        .filter(|query| query.text.is_none() && query.entity.is_some())
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].entity.as_deref(), Some("Mira"));
    assert!(related[0].limit <= 8);
    stop(client).await;
}

#[tokio::test]
async fn unsupported_primary_quotes_cannot_trigger_related_history_reads() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    colleague(&store);
    store.facts.lock().unwrap()[0]
        .extraction
        .as_mut()
        .unwrap()
        .evidence = "Invented source quote".into();
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "unsupported".into(),
            prompt: "Which breakfast drink?".into(),
        })
        .unwrap();
    event(&client, |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "unsupported")).await;
    assert!(!memory_request(&server).contains("memory:fact:tea"));
    let evidence = first_evidence(&server);
    assert_eq!(evidence["memories"], json!([]));
    assert!(
        evidence["background"]
            .to_string()
            .contains("memory:fact:colleague")
    );
    assert!(
        !store
            .recalls
            .lock()
            .unwrap()
            .iter()
            .any(|query| query.entity.is_some())
    );
    stop(client).await;
}

#[tokio::test]
async fn slow_related_reads_keep_primary_evidence_and_unrelated_questions_do_not_expand() {
    let server = recall_server();
    let store = Arc::new(Store::default());
    colleague(&store);
    let (_release, gate) = async_channel::bounded(1);
    *store.related_gate.lock().unwrap() = Some(gate);
    let (_fixture, client) = start(&server, store.clone(), true).await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "slow".into(),
            prompt: "Which breakfast drink?".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "slow"),
    )
    .await;
    let messages = memory_request(&server);
    assert!(messages.contains("memory:fact:tea"));
    let evidence = first_evidence(&server);
    assert!(
        !evidence["memories"]
            .to_string()
            .contains("memory:fact:colleague")
    );
    assert!(
        evidence["background"]
            .to_string()
            .contains("memory:fact:colleague")
    );
    let before = store
        .recalls
        .lock()
        .unwrap()
        .iter()
        .filter(|query| query.entity.is_some())
        .count();
    assert_eq!(before, 1);
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "unrelated".into(),
            prompt: "Explain black holes".into(),
        })
        .unwrap();
    event(
        &client,
        |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "unrelated"),
    )
    .await;
    assert_eq!(
        store
            .recalls
            .lock()
            .unwrap()
            .iter()
            .filter(|query| query.entity.is_some())
            .count(),
        before
    );
    stop(client).await;
}

#[tokio::test]
async fn pause_and_withdrawal_revoke_context_while_a_related_read_is_held() {
    for withdraw in [false, true] {
        let server = recall_server();
        let (started, observed) = async_channel::bounded(1);
        let store = Arc::new(Store {
            related_started: Some(started),
            ..Default::default()
        });
        colleague(&store);
        let (_release, gate) = async_channel::bounded(1);
        *store.related_gate.lock().unwrap() = Some(gate);
        let (_fixture, client) = start(&server, store.clone(), true).await;
        client
            .send(Command::RunPrompt {
                context: None,
                session_id: "held".into(),
                prompt: "Which breakfast drink?".into(),
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), observed.recv())
            .await
            .unwrap()
            .unwrap();
        if withdraw {
            client
                .send(Command::Memory(sovra_core::MemoryCommand {
                    request_id: 1,
                    action: sovra_core::MemoryAction::StopUsing(FactId("fact:colleague".into())),
                }))
                .unwrap();
        } else {
            send(
                &client,
                ModelCommand::SetAgentSettings {
                    settings: AgentSettings {
                        memory_enabled: false,
                        ..Default::default()
                    },
                },
            );
        }
        // The held read is never released. Cancellation must reach the provider boundary.
        event(
            &client,
            |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "held"),
        )
        .await;
        let messages = memory_request(&server);
        assert!(!messages.contains("memory:fact:tea"));
        assert!(!messages.contains("memory:fact:colleague"));
        stop(client).await;
    }
}
