//! Local models with a 32K window must be able to chat: Sovra's own instructions
//! and tool descriptions are well under that many tokens.

use super::*;

#[tokio::test]
async fn a_32k_local_model_answers_a_greeting() {
    let server = Server::concurrent(move |request| match request.path.as_str() {
        "/api/tags" => Reply::json(&json!({"models":[{"model":"qwen2.5-coder:7b"}]})),
        "/api/show" => Reply::json(
            &json!({"capabilities":["completion","tools","insert"],"model_info":{"qwen2.context_length":32_768}}),
        ),
        "/api/version" | "/" => Reply::json(&json!({"version":"0.12.0"})),
        "/v1/chat/completions" => {
            let chunk = |text: &str, finish: Value| {
                format!(
                    "data: {}\n\n",
                    json!({"id":"s","object":"chat.completion.chunk","created":1,"model":"qwen2.5-coder:7b",
                        "choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":finish}]})
                )
            };
            Reply {
                status: 200,
                content_type: "text/event-stream",
                body: format!(
                    "{}{}data: [DONE]\n\n",
                    chunk("Hello!", Value::Null),
                    chunk("", json!("stop"))
                ),
                tail: None,
            }
        }
        other => panic!("unexpected path {other}"),
    });
    let mut connection = ModelConnection::new("ollama", ProviderKind::Ollama);
    connection.base_url = server.url();
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    let snapshot = catalog(&client).await;
    assert_eq!(snapshot.models[0].context_length, Some(32_768));
    let key = ModelKey {
        connection_id: "ollama".into(),
        model_id: "qwen2.5-coder:7b".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key))
    })
    .await;
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "small".into(),
            prompt: "hello!".into(),
        })
        .unwrap();
    let outcome = event(&client, |e| {
        matches!(e, Event::AgentChunk { .. } | Event::AgentError { .. })
    })
    .await;
    assert!(matches!(outcome, Event::AgentChunk { .. }), "{outcome:?}");
    stop(client).await;
}
