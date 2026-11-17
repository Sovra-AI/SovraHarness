//! Several chats answer at once: a slow reply in one never holds up another.

use super::*;

fn chunk(text: &str, finish: Option<&str>) -> String {
    let delta = if finish.is_some() {
        json!({})
    } else {
        json!({"role":"assistant","content":text})
    };
    format!(
        "data: {}\n\n",
        json!({"id":"c","object":"chat.completion.chunk","created":1,"model":"local-chat",
            "choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}

#[tokio::test]
async fn a_second_chat_answers_while_the_first_is_still_replying() {
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = Arc::new(Mutex::new(gate));
    let server = Server::concurrent(move |request| {
        if request.path == "/v1/models" {
            return Reply::json(&json!({"data":[{"id":"local-chat"}]}));
        }
        let slow = request.body.to_string().contains("slow-question");
        let (head, rest) = if slow {
            ("Slow start", "Slow end")
        } else {
            ("Quick answer", "")
        };
        let tail = format!(
            "{}{}data: [DONE]\n\n",
            chunk(rest, None),
            chunk("", Some("stop"))
        );
        if slow {
            // The slow chat's reply stays open until the test releases it.
            Reply {
                status: 200,
                content_type: "text/event-stream",
                body: chunk(head, None),
                tail: Some((gate.clone(), tail)),
            }
        } else {
            Reply {
                status: 200,
                content_type: "text/event-stream",
                body: format!("{}{tail}", chunk(head, None)),
                tail: None,
            }
        }
    });
    let mut connection = ModelConnection::new("custom", ProviderKind::OpenAiCompatible);
    connection.base_url = format!("{}/v1", server.url());
    connection.authentication = Authentication::None;
    let fixture = Fixture::new(connection, None);
    let client = fixture.start();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "custom".into(),
        model_id: "local-chat".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key))
    })
    .await;

    let ask = |session: &str, prompt: &str| {
        client
            .send(Command::RunPrompt {
                context: None,
                session_id: session.into(),
                prompt: prompt.into(),
            })
            .unwrap();
    };
    ask("first", "slow-question");
    // Only ask the second chat once the first is visibly mid-reply.
    let started = event(&client, |e| {
        matches!(e, Event::AgentChunk { session_id, .. } | Event::AgentError { session_id, .. } if session_id == "first")
    })
    .await;
    assert!(
        matches!(&started, Event::AgentChunk { text, .. } if text.contains("Slow start")),
        "{started:?}"
    );
    ask("second", "quick-question");
    // The second chat must finish while the first chat's reply is still held open.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match client.events().recv().await.unwrap() {
                Event::AgentCompleted { session_id } if session_id == "first" => {
                    panic!("the slow chat finished before it was released");
                }
                Event::AgentError {
                    session_id,
                    message,
                } => {
                    panic!("{session_id} failed: {message}");
                }
                Event::AgentCompleted { session_id } if session_id == "second" => break,
                _ => {}
            }
        }
    })
    .await
    .expect("the second chat must answer while the first is still replying");

    release.send(()).unwrap();
    event(
        &client,
        |e| matches!(e, Event::AgentCompleted { session_id } if session_id == "first"),
    )
    .await;
    stop(client).await;
}

/// A hosted connection on the loopback server: local addresses always get at
/// least ten minutes, so only a hosted kind keeps the short timeout under test.
async fn hosted(server: &Server, timeout_seconds: u64) -> (Fixture, Client) {
    let mut connection = ModelConnection::new("hosted", ProviderKind::OpenRouter);
    connection.base_url = format!("{}/api/v1", server.url());
    connection.timeout_seconds = timeout_seconds;
    connection.has_credential = true;
    let fixture = Fixture::new(connection, Some("hosted-test-key"));
    let client = fixture.start();
    catalog(&client).await;
    let key = ModelKey {
        connection_id: "hosted".into(),
        model_id: "local-chat".into(),
    };
    send(&client, ModelCommand::Activate { key: key.clone() });
    event(&client, |e| {
        matches!(e, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.as_ref() == Some(&key))
    })
    .await;
    (fixture, client)
}

fn hosted_models(request: &Request) -> Option<Reply> {
    request
        .path
        .starts_with("/api/v1/models")
        .then(|| Reply::json(&json!({"data":[{"id":"local-chat"}]})))
}

/// Regression: the connection timeout capped the whole turn, so a reply that kept
/// streaming (or a turn running many web searches next to other chats) was cut
/// off with "timed out" while it was still working.
#[tokio::test]
async fn a_reply_that_keeps_streaming_is_not_cut_off_by_the_connection_timeout() {
    const PIECES: usize = 6;
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = Arc::new(Mutex::new(gate));
    let server = Server::concurrent(move |request| {
        if let Some(reply) = hosted_models(request) {
            return reply;
        }
        assert_eq!(request.path, "/api/v1/chat/completions");
        let mut tail: Vec<String> = (1..PIECES)
            .map(|n| chunk(&format!(" part{n}"), None))
            .collect();
        tail.push(format!("{}data: [DONE]\n\n", chunk("", Some("stop"))));
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body: chunk("part0", None),
            tail: Some((gate.clone(), tail.join(&http::TAIL_PART.to_string()))),
        }
    });
    // Five seconds of allowed silence; the whole reply takes about twelve.
    let (_fixture, client) = hosted(&server, 5).await;
    // Releases one more piece of the reply every two seconds.
    let pacer = async move {
        for _ in 0..PIECES {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if release.send(()).is_err() {
                break;
            }
        }
    };
    let started = std::time::Instant::now();
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "slow".into(),
            prompt: "Tell me something long".into(),
        })
        .unwrap();
    let mut text = String::new();
    let reply = async {
        loop {
            match client.events().recv().await.unwrap() {
                Event::AgentChunk { text: piece, .. } => text.push_str(&piece),
                Event::AgentError { message, .. } => panic!("the reply was cut off: {message}"),
                Event::AgentCompleted { .. } => break,
                _ => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(pacer, reply)
    })
    .await
    .expect("the streaming reply must finish");
    assert!(
        started.elapsed() > Duration::from_secs(10),
        "the reply must outlast the timeout"
    );
    assert_eq!(text, "part0 part1 part2 part3 part4 part5");
    stop(client).await;
}

/// The other half of the rule: a model that goes silent is still stopped.
#[tokio::test]
async fn a_model_that_stops_sending_is_still_stopped_after_the_timeout() {
    let (_never, gate) = std::sync::mpsc::channel::<()>();
    let gate = Arc::new(Mutex::new(gate));
    let server = Server::concurrent(move |request| {
        if let Some(reply) = hosted_models(request) {
            return reply;
        }
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body: chunk("Hello", None),
            tail: Some((
                gate.clone(),
                format!("{}data: [DONE]\n\n", chunk("", Some("stop"))),
            )),
        }
    });
    let (_fixture, client) = hosted(&server, 5).await;
    let started = std::time::Instant::now();
    client
        .send(Command::RunPrompt {
            context: None,
            session_id: "silent".into(),
            prompt: "Hello".into(),
        })
        .unwrap();
    let failed = event(&client, |e| {
        matches!(e, Event::AgentError { .. } | Event::AgentCompleted { .. })
    })
    .await;
    assert!(
        matches!(&failed, Event::AgentError { message, .. } if message.contains("stopped responding")),
        "{failed:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    stop(client).await;
}
