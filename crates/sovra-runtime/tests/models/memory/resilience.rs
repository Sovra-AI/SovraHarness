//! Independent retrieval failures must not discard successful permitted evidence.

use super::*;

#[tokio::test]
async fn source_search_failure_or_timeout_preserves_grounded_fact_recall() {
    for delayed in [false, true] {
        let server = recall_server();
        let (_release, gate) = async_channel::bounded(1);
        let store = Arc::new(Store {
            source_search_gate: delayed.then_some(gate),
            source_search_failure: !delayed,
            ..Default::default()
        });
        saved_tea(&store);
        let (_fixture, client) = start(&server, store, true).await;
        client
            .send(Command::RunPrompt {
                context: Some(sovra_core::ConversationContext::default()),
                session_id: "partial-recall".into(),
                prompt: "Which tea do I prefer?".into(),
            })
            .unwrap();
        event(&client, |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "partial-recall")).await;
        {
            let requests = server.requests.lock().unwrap();
            let request = requests
                .iter()
                .find(|request| request.path.ends_with("completions") && !is_extraction(request))
                .unwrap();
            let messages = request.body["messages"].to_string();
            assert!(
                messages.contains("memory:fact:tea"),
                "a failed independent source leg must not discard grounded facts: {messages}"
            );
            let evidence = related::evidence_sections(&request.body);
            assert!(
                evidence["notice"]
                    .as_str()
                    .is_some_and(|notice| !notice.is_empty())
            );
        }
        stop(client).await;
    }
}

#[tokio::test]
async fn removal_during_a_slow_source_search_discards_already_read_facts() {
    let server = recall_server();
    let (_release, gate) = async_channel::bounded(1);
    let (started, observed) = async_channel::bounded(1);
    let store = Arc::new(Store {
        source_search_gate: Some(gate),
        source_search_started: Some(started),
        ..Default::default()
    });
    saved_tea(&store);
    let (_fixture, client) = start(&server, store, true).await;
    client
        .send(Command::RunPrompt {
            context: Some(sovra_core::ConversationContext::default()),
            session_id: "withdraw-during-partial-recall".into(),
            prompt: "Which tea do I prefer?".into(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), observed.recv())
        .await
        .unwrap()
        .unwrap();
    client
        .send(Command::Memory(sovra_core::MemoryCommand {
            request_id: 1,
            action: sovra_core::MemoryAction::Remove(FactId("fact:tea".into())),
        }))
        .unwrap();
    event(&client, |event| matches!(event, Event::AgentCompleted { session_id } if session_id == "withdraw-during-partial-recall")).await;
    {
        let requests = server.requests.lock().unwrap();
        let request = requests
            .iter()
            .find(|request| request.path.ends_with("completions") && !is_extraction(request))
            .unwrap();
        assert!(
            !request.body["messages"]
                .to_string()
                .contains("memory:fact:tea")
        );
    }
    stop(client).await;
}
