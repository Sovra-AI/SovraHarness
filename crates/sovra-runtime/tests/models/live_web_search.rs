//! Opt-in live check of the app's own chat path, driven through `Client` exactly
//! as the UI drives it: Ollama connection → catalog → activate model → chat turn
//! → orchestrator → `WebSearchAgent` → built-in search → streamed answer.
//!
//! A local stand-in `SearXNG` (the app's optional extra source) records every
//! query, so the test proves a search really ran in each turn rather than the
//! model answering from memory.
//!
//! `cargo test -p sovra-runtime --test models live_ -- --ignored --nocapture`
//! (model: `SOVRA_LIVE_MODEL`, default `qwen2.5-coder:7b`).

use super::*;

const MARKER_URL: &str = "https://searx-standin.example/rust-release";

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Ollama model and the internet"]
#[expect(
    clippy::disallowed_methods,
    reason = "opt-in live test picks its model from the environment"
)]
async fn live_chat_turn_searches_the_web_through_the_runtime() {
    let model = std::env::var("SOVRA_LIVE_MODEL").unwrap_or_else(|_| "qwen2.5-coder:7b".into());
    let searx = Server::new(|request| {
        assert!(request.path.starts_with("/search?"), "{}", request.path);
        Reply::json(&json!({"results": [{
            "title": "Rust release notes (stand-in source)",
            "url": MARKER_URL,
            "content": "Release history of the Rust programming language."
        }]}))
    });
    let mut connection = ModelConnection::new("ollama", ProviderKind::Ollama);
    connection.base_url = "http://127.0.0.1:11434".into();
    connection.authentication = Authentication::None;
    let fixture = Fixture::new(connection, None);
    let config = AgentConfig {
        searxng_base_url: Some(searx.url()),
        ..AgentConfig::default()
    };
    let client = sovra_runtime::start_with_models(
        "live",
        config,
        None,
        Some(ModelsConfig {
            readiness: None,
            repository: fixture.repo.clone(),
            providers: vec![Arc::new(OllamaProvider)],
        }),
    )
    .unwrap();
    catalog(&client).await;
    send(
        &client,
        ModelCommand::Activate {
            key: ModelKey {
                connection_id: "ollama".into(),
                model_id: model.clone(),
            },
        },
    );
    event(&client, |event| {
        matches!(event, Event::Models(ModelEvent::PreferencesChanged(p)) if p.active.is_some())
    })
    .await;

    for (session, prompt) in [
        (
            "live-explicit",
            "Search the web for the latest stable release of the Rust programming language.",
        ),
        (
            "live-natural",
            "What is the latest stable release of the Rust programming language?",
        ),
    ] {
        let searches_before = searx.requests.lock().unwrap().len();
        client
            .send(Command::RunPrompt {
                context: None,
                session_id: session.into(),
                prompt: prompt.into(),
            })
            .unwrap();
        let answer = tokio::time::timeout(Duration::from_secs(300), async {
            let mut answer = String::new();
            loop {
                match client.events().recv().await.unwrap() {
                    Event::AgentChunk { session_id, text } if session_id == session => {
                        answer.push_str(&text);
                    }
                    Event::AgentCompleted { session_id, .. } if session_id == session => {
                        return Ok(answer);
                    }
                    Event::AgentError {
                        session_id,
                        message,
                    } if session_id == session => return Err(message),
                    _ => {}
                }
            }
        })
        .await
        .expect("turn finished in time")
        .unwrap_or_else(|message| panic!("{prompt}: chat turn failed: {message}"));
        let searches = searx.requests.lock().unwrap().len() - searches_before;
        eprintln!("--- {prompt}\nsearches: {searches}\n{answer}\n");
        assert!(
            searches > 0,
            "{prompt}: no search ran; the answer came from the model's memory:\n{answer}"
        );
        assert!(
            answer.contains("https://") || answer.contains("http://"),
            "{prompt}: the answer cites no web source:\n{answer}"
        );
    }
    stop(client).await;
}
