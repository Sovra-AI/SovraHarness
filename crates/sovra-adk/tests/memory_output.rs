//! Verify extraction format controls reach the pinned provider transports.

#![allow(
    clippy::unwrap_used,
    reason = "integration fixture failures must fail the test"
)]

#[path = "../../../tests/support/model_http.rs"]
mod http;

use http::{Reply, Server};
use serde_json::json;
use sovra_adk::AdkHarness;
use sovra_core::{
    AgentConfig, Episode, EpisodeId, EpisodeKind, Fact, FactId, Timestamp, models::*,
};

#[tokio::test]
async fn advertised_memory_schema_reaches_the_wire_without_affecting_chat() {
    for kind in [
        ProviderKind::OpenRouter,
        ProviderKind::OpenAiCompatible,
        ProviderKind::Ollama,
    ] {
        for supported in [false, true] {
            let server = Server::new(|request| {
                let extraction = request.body["messages"]
                    .to_string()
                    .contains("Extract durable");
                let output = if extraction {
                    r#"{"memories":[]}"#
                } else {
                    "Noted."
                };
                Reply::json(
                    &json!({"id":"fixture","object":"chat.completion","created":1,"model":"format-fixture",
                    "choices":[{"index":0,"message":{"role":"assistant","content":output},"finish_reason":"stop"}]}),
                )
            });
            let mut connection = ModelConnection::new("fixture", kind);
            connection.base_url = if kind == ProviderKind::Ollama {
                server.url()
            } else {
                format!("{}/v1", server.url())
            };
            connection.authentication = Authentication::None;
            let model = ModelInfo {
                id: "format-fixture".into(),
                supported_parameters: if supported {
                    vec!["structured_outputs".into()]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            let harness = AdkHarness::from_selected_model(
                &AgentConfig::default(),
                &connection,
                &model,
                &ModelSecret::default(),
            )
            .unwrap()
            .without_web_search()
            .unwrap();
            let source = Episode {
                id: EpisodeId("episode:fixture".into()),
                session_id: "source".into(),
                kind: EpisodeKind::UserMessage,
                content: "What is my favorite composer?".into(),
                occurred_at: Timestamp(1_000),
            };
            assert_eq!(
                harness.extract_memories(&source, &[]).await.unwrap(),
                Vec::<sovra_core::MemoryCandidate>::new()
            );
            if supported {
                let known = Fact {
                    id: FactId("fact:provided".into()),
                    content: "The user prefers tea.".into(),
                    entities: vec!["user".into()],
                    source: None,
                    replaces: None,
                    extraction: None,
                    created_at: Timestamp(1),
                    valid_at: None,
                    invalid_at: None,
                    expired_at: None,
                    stopped_at: None,
                };
                assert_eq!(
                    harness.extract_memories(&source, &[known]).await.unwrap(),
                    Vec::<sovra_core::MemoryCandidate>::new()
                );
            }
            let (events, received) = async_channel::unbounded();
            harness.run_prompt("chat", "Hello", &events).await;
            while let Ok(event) = received.try_recv() {
                assert!(
                    !matches!(event, sovra_core::Event::AgentError { .. }),
                    "{event:?}"
                );
            }
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), if supported { 3 } else { 2 });
            assert_eq!(requests[0].path, "/v1/chat/completions");
            assert!(
                requests[0]
                    .headers
                    .to_ascii_lowercase()
                    .contains("content-type: application/json")
            );
            let extraction = &requests[0].body;
            if supported {
                assert_eq!(
                    extraction["response_format"]["type"], "json_schema",
                    "{kind:?}"
                );
                assert_eq!(extraction["response_format"]["json_schema"]["strict"], true);
                let schema = &extraction["response_format"]["json_schema"]["schema"];
                assert_eq!(schema["additionalProperties"], false);
                let item = &schema["properties"]["memories"]["items"];
                assert_eq!(item["additionalProperties"], false);
                assert_eq!(item["properties"]["replaces"], json!({"type":"null"}));
                assert_eq!(item["required"].as_array().unwrap().len(), 6);
                let known_properties = &requests[1].body["response_format"]["json_schema"]["schema"]
                    ["properties"]["memories"]["items"]["properties"];
                for field in ["replaces", "same_as"] {
                    assert_eq!(
                        known_properties[field],
                        json!({"anyOf":[{"type":"string","enum":["fact:provided"]},{"type":"null"}]})
                    );
                }
                if kind == ProviderKind::OpenRouter {
                    assert_eq!(extraction["provider"]["require_parameters"], true);
                    assert!(
                        extraction.get("temperature").is_none(),
                        "unadvertised sampling parameters must not defeat required-parameter routing"
                    );
                }
            } else {
                assert!(extraction.get("response_format").is_none());
            }
            let chat = &requests.last().unwrap().body;
            assert!(chat.get("response_format").is_none());
            assert!(chat.get("provider").is_none());
        }
    }
}
