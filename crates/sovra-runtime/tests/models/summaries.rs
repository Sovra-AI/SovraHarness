//! Summary HTTP fixtures run on the runtime's executor; providers own no runtime.

use super::*;
use sovra_core::file_index::{PassageRef, summary::*};
use sovra_providers::HttpSummarizer;

fn setup(server: &Server, kind: ProviderKind) -> (Fixture, PreparedSummary) {
    let mut connection = ModelConnection::new("summary", kind);
    connection.base_url = server.url();
    connection.has_credential = true;
    connection.authentication = if kind == ProviderKind::Anthropic {
        Authentication::ApiKey
    } else {
        Authentication::Bearer
    };
    let model = ModelInfo {
        id: "summarizer".into(),
        context_length: Some(32_768),
        ..ModelInfo::default()
    };
    connection.manual_models.push(model.clone());
    let connection = sovra_providers::validate_connection(&connection).unwrap();
    let target = ModelDestination::for_work(&connection, &model, ModelWork::Summaries).unwrap();
    let fixture = Fixture::new(connection, Some("summary-test-key"));
    let reference = SummaryInputRef::Passage(PassageRef {
        root_id: "folder".into(),
        file_id: "file".into(),
        version_id: "version".into(),
        ordinal: 0,
    });
    let prepared = PreparedSummary {
        configuration: SummaryConfiguration {
            id: 1,
            plan: SummaryPlan {
                name: "Map".into(),
                roots: vec!["folder".into()],
                summarizer: target.clone(),
                embedder: target,
                max_calls: 10,
                automatic: false,
            },
            enabled: true,
            created_at: 1,
            calls_used: 0,
        },
        request: SummaryRequest {
            configuration_id: 1,
            kind: SummaryKind::Document,
            inputs: vec![reference.clone()],
        },
        inputs: vec![SummaryInput {
            reference,
            label: "plan.txt".into(),
            text: "A proposed garden uses orchids.".into(),
            extraction_note: Some("Partial extraction.".into()),
        }],
        level: 1,
        content_key: "prepared-key".into(),
    };
    (fixture, prepared)
}

fn response(kind: ProviderKind) -> Value {
    let text = json!({"title":"Garden proposal","claims":[{"statement":"The proposed garden uses orchids.","supports":[0]}],"caveats":["Partial extraction."]}).to_string();
    match kind {
        ProviderKind::Ollama => {
            json!({"message":{"role":"assistant","content":text},"done":true,"done_reason":"stop"})
        }
        ProviderKind::OpenAiCompatible | ProviderKind::OpenRouter => {
            json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":text}}]})
        }
        ProviderKind::Anthropic => {
            json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]})
        }
    }
}

#[tokio::test]
async fn summaries_use_the_approved_protocol_and_credentials_once_per_adapter() {
    for kind in [
        ProviderKind::Ollama,
        ProviderKind::OpenAiCompatible,
        ProviderKind::OpenRouter,
        ProviderKind::Anthropic,
    ] {
        let server = Server::new(move |request| {
            let (path, header) = match kind {
                ProviderKind::Ollama => ("/api/chat", "authorization: bearer summary-test-key"),
                ProviderKind::OpenAiCompatible => (
                    "/v1/chat/completions",
                    "authorization: bearer summary-test-key",
                ),
                ProviderKind::OpenRouter => (
                    "/api/v1/chat/completions",
                    "authorization: bearer summary-test-key",
                ),
                ProviderKind::Anthropic => ("/v1/messages", "x-api-key: summary-test-key"),
            };
            assert_eq!(request.path, path);
            assert!(request.headers.to_ascii_lowercase().contains(header));
            assert_eq!(request.body["model"], "summarizer");
            assert!(request.body.get("tools").is_none());
            Reply::json(&response(kind))
        });
        let (fixture, prepared) = setup(&server, kind);
        let adapter =
            HttpSummarizer::from_repository(&fixture.repo, &prepared.configuration.plan.summarizer)
                .unwrap();
        assert!(!format!("{adapter:?}").contains("summary-test-key"));
        let result = adapter.summarize(&prepared).await.unwrap();
        assert_eq!(result.claims[0].supports, [0]);
        assert_eq!(result.caveats, ["Partial extraction."]);
        assert!(adapter.summarize(&prepared).await.is_err());
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn changed_destinations_and_invalid_prepared_inputs_send_nothing() {
    let server = Server::new(|_| panic!("Invalid input must never reach HTTP"));
    let (fixture, prepared) = setup(&server, ProviderKind::OpenAiCompatible);
    let target = &prepared.configuration.plan.summarizer;
    let adapter = HttpSummarizer::from_repository(&fixture.repo, target).unwrap();
    let mut bad = prepared.clone();
    bad.configuration
        .plan
        .summarizer
        .base_url
        .push_str("/changed");
    assert!(adapter.summarize(&bad).await.is_err());
    bad = prepared.clone();
    bad.inputs[0].text = "x".repeat(MAX_SUMMARY_INPUT_BYTES);
    assert!(adapter.summarize(&bad).await.is_err());
    let mut saved = fixture.repo.load().unwrap();
    saved.connections[0].manual_models[0].context_length = None;
    fixture.repo.save(&saved).unwrap();
    let unknown = HttpSummarizer::from_repository(&fixture.repo, target).unwrap();
    assert!(unknown.summarize(&prepared).await.is_err());
    saved.connections[0].base_url.push_str("/changed");
    fixture.repo.save(&saved).unwrap();
    assert!(HttpSummarizer::from_repository(&fixture.repo, target).is_err());
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn summary_http_failures_are_bounded_redacted_and_never_retried() {
    for (status, body) in [
        (401, "private server error".into()),
        (200, "private invalid JSON".into()),
        (200, "x".repeat(128 * 1024 + 1)),
    ] {
        let server = Server::new(move |_| Reply {
            status,
            content_type: "application/json",
            body: body.clone(),
            tail: None,
        });
        let (fixture, prepared) = setup(&server, ProviderKind::OpenAiCompatible);
        let adapter =
            HttpSummarizer::from_repository(&fixture.repo, &prepared.configuration.plan.summarizer)
                .unwrap();
        let error = adapter.summarize(&prepared).await.unwrap_err().to_string();
        assert!(!error.contains("private"));
        assert!(!error.contains("summary-test-key"));
        assert!(adapter.summarize(&prepared).await.is_err());
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn cancelling_a_partial_summary_does_not_allow_an_unreserved_retry() {
    let (release, gate) = std::sync::mpsc::channel();
    let gate = Arc::new(Mutex::new(gate));
    let server = Server::new(move |_| Reply {
        status: 200,
        content_type: "application/json",
        body: "{".into(),
        tail: Some((gate.clone(), "}".into())),
    });
    let (fixture, prepared) = setup(&server, ProviderKind::Ollama);
    let adapter =
        HttpSummarizer::from_repository(&fixture.repo, &prepared.configuration.plan.summarizer)
            .unwrap();
    let mut pending = Box::pin(adapter.summarize(&prepared));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut pending => panic!("Response should wait for its tail: {result:?}"),
                () = tokio::task::yield_now() => {
                    if !server.requests.lock().unwrap().is_empty() { break; }
                }
            }
        }
    })
    .await
    .unwrap();
    // Dropping the future is the same cancellation mechanism used by runtime tasks.
    drop(pending);
    assert!(adapter.summarize(&prepared).await.is_err());
    release.send(()).unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}
