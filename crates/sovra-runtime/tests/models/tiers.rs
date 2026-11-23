//! Choosing the AI by question difficulty, end to end through `Client` and real HTTP.

use super::*;
use sovra_core::ai_tiers::{DecisionSource, ThinkingLevel, TierChoice, TierRouting};
use sovra_core::providers::ModelTier;

fn reply(model: &str) -> Reply {
    let chunk = |text: &str, finish: Value| {
        format!(
            "data: {}\n\n",
            json!({
                "id":"tiers", "object":"chat.completion.chunk", "created":1, "model":model,
                "choices":[{"index":0, "delta":{"role":"assistant","content":text}, "finish_reason":finish}]
            })
        )
    };
    Reply {
        status: 200,
        content_type: "text/event-stream",
        body: format!(
            "{}{}data: [DONE]\n\n",
            chunk(&format!("from {model}"), Value::Null),
            chunk("", json!("stop"))
        ),
        tail: None,
    }
}

fn key(model: &str) -> ModelKey {
    ModelKey {
        connection_id: "local".into(),
        model_id: model.into(),
    }
}

/// Runs one message and returns who answered it.
async fn ask(client: &Client, session: &str, prompt: &str) -> sovra_core::ResponseModel {
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: session.into(),
            prompt: prompt.into(),
        })
        .unwrap();
    let started = event(
        client,
        |e| matches!(e, Event::AgentResponseStarted { session_id, .. } if session_id == session),
    )
    .await;
    event(
        client,
        |e| matches!(e, Event::AgentCompleted { session_id } if session_id == session),
    )
    .await;
    match started {
        Event::AgentResponseStarted { model, .. } => model,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn each_difficulty_goes_to_its_own_model_at_its_own_thinking_level() {
    let server = Server::concurrent(|request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"small"}, {"id":"big"}]}));
        }
        reply(request.body["model"].as_str().unwrap_or("unknown"))
    });
    let mut connection = ModelConnection::new("local", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    send(&client, ModelCommand::Activate { key: key("small") });
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active == Some(key("small")))
    })
    .await;

    // Sorting off: the chat model answers everything, with no thinking setting sent.
    let single = ask(&client, "off", "Design an algorithm to schedule nurses across three shifts and compare it with a greedy approach").await;
    assert_eq!(
        (single.model_id.as_str(), single.route.as_ref()),
        ("small", None)
    );

    let mut routing = TierRouting {
        enabled: true,
        ..TierRouting::default()
    };
    routing.tiers.insert(
        ModelTier::Deep,
        TierChoice {
            model: Some(key("big")),
            thinking: ThinkingLevel::High,
        },
    );
    routing.tiers.insert(
        ModelTier::Fast,
        TierChoice {
            model: None,
            thinking: ThinkingLevel::Off,
        },
    );
    send(
        &client,
        ModelCommand::SetTierRouting {
            routing: routing.clone(),
        },
    );
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.tier_routing == routing)
    })
    .await;

    let simple = ask(&client, "simple", "hi").await;
    assert_eq!(simple.model_id, "small");
    assert!(
        simple
            .route
            .as_deref()
            .is_some_and(|r| r.starts_with("Simple question · No thinking")),
        "{simple:?}"
    );
    let complex = ask(
        &client,
        "complex",
        "Why does this fail?\n```rust\nfn main() { let x: u8 = 300; }\n```",
    )
    .await;
    assert_eq!(complex.model_id, "big");
    assert!(
        complex
            .route
            .as_deref()
            .is_some_and(|r| r.starts_with("Complex question · Think hard")),
        "{complex:?}"
    );

    let chats: Vec<Value> = server
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path.ends_with("/chat/completions"))
        .map(|r| r.body.clone())
        .collect();
    let sent = |model: &str| {
        chats
            .iter()
            .filter(|body| body["model"] == model)
            .map(|body| body["reasoning_effort"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(sent("small"), vec![Value::Null, json!("none")]);
    assert_eq!(sent("big"), vec![json!("high")]);

    // The sample tester sorts without contacting any AI.
    let before = server.requests.lock().unwrap().len();
    send(
        &client,
        ModelCommand::TestTierDecision {
            prompt: "Quickly: capital of Peru?".into(),
        },
    );
    let tested = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::TierTested { .. }))
    })
    .await;
    assert!(matches!(
        tested,
        Event::Models(ModelEvent::TierTested { decision, .. })
            if decision.tier == ModelTier::Fast && decision.source == DecisionSource::Asked
    ));
    assert_eq!(server.requests.lock().unwrap().len(), before);

    // Laya is not part of this runtime, so asking for it is a plain error.
    send(&client, ModelCommand::InstallLaya);
    let refused = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Error(_)))
    })
    .await;
    assert!(matches!(refused, Event::Models(ModelEvent::Error(m)) if m.contains("not available")));

    stop(client).await;
    let client = fixture.start();
    let loaded = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Loaded { .. }))
    })
    .await;
    assert!(
        matches!(loaded, Event::Models(ModelEvent::Loaded { preferences, .. }) if preferences.tier_routing == routing)
    );
    stop(client).await;
}

#[tokio::test]
async fn a_tier_model_that_is_not_a_chat_model_is_refused() {
    let server = Server::new(|_| {
        Reply::json(&json!({"data":[
            {"id":"chat"},
            {"id":"batch:batch", "architecture":{"output_modalities":["text"]}}
        ]}))
    });
    let mut connection = ModelConnection::new("local", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    let mut routing = TierRouting {
        enabled: true,
        ..TierRouting::default()
    };
    routing.tiers.insert(
        ModelTier::Deep,
        TierChoice {
            model: Some(key("batch:batch")),
            thinking: ThinkingLevel::Max,
        },
    );
    send(&client, ModelCommand::SetTierRouting { routing });
    let refused = event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::Error(_)))
    })
    .await;
    assert!(matches!(refused, Event::Models(ModelEvent::Error(m)) if m.contains("batch")));
    stop(client).await;
}
