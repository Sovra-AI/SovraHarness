//! Exercise the actual ADK tool loop with deterministic local model responses.

use crate::AdkHarness;
use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use serde_json::json;
use sovra_core::{AgentConfig, Event};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct FileModel {
    tools_enabled: bool,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl Llm for FileModel {
    fn name(&self) -> &'static str {
        "file-reader-test"
    }

    async fn generate_content(
        &self,
        request: LlmRequest,
        _: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        assert_eq!(
            request.tools.contains_key("read_attachment"),
            self.tools_enabled
        );
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let last_result = request
            .contents
            .iter()
            .rev()
            .flat_map(|content| content.parts.iter().rev())
            .find_map(|part| {
                if let Part::FunctionResponse {
                    function_response, ..
                } = part
                {
                    Some(&function_response.response)
                } else {
                    None
                }
            });
        let part = if self.tools_enabled {
            match call {
                0 => call_tool(json!({"file_id":"not-attached", "query":"needle"}), call),
                1 => {
                    assert!(last_result.unwrap().get("error").is_some());
                    call_tool(
                        json!({"file_id":"picture-1", "query":"needle", "result_offset":3}),
                        call,
                    )
                }
                2 => {
                    let result = last_result.unwrap();
                    assert_eq!(result["sections"][0]["section"], 4);
                    assert!(result["sections"][0]["start_line"].as_u64().unwrap() > 1);
                    assert!(
                        result["sections"][0]["text"]
                            .as_str()
                            .unwrap()
                            .contains("needle")
                    );
                    Part::Text {
                        text: "Read page two with line citations.".into(),
                    }
                }
                _ => panic!("Unexpected extra file-reader model call"),
            }
        } else {
            assert!(
                request.tools.is_empty(),
                "A model without tool support must receive no tool declarations"
            );
            assert_eq!(call, 0);
            assert!(request.contents.iter().flat_map(|content| &content.parts).any(|part| matches!(part, Part::Text { text } if text.contains("needle") && text.contains("lines "))));
            Part::Text {
                text: "Read the supplied excerpt with tools disabled.".into(),
            }
        };
        let response = LlmResponse {
            content: Some(Content {
                role: "model".into(),
                parts: vec![part],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

fn call_tool(args: serde_json::Value, call: usize) -> Part {
    Part::FunctionCall {
        name: "read_attachment".into(),
        args,
        id: Some(format!("file-call-{call}")),
        thought_signature: None,
    }
}

#[test]
fn agent_reads_later_search_pages_and_respects_disabled_tools() {
    // ADK times tool calls with tokio; use the existing test-only reactor.
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            for enabled in [true, false] {
                let llm = Arc::new(FileModel {
                    tools_enabled: enabled,
                    calls: AtomicUsize::new(0),
                });
                let base =
                    AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm.clone(), vec![])
                        .unwrap();
                let mut previous = super::tests::fixture();
                previous.attachment.id = "not-attached".into();
                let base = base.with_files(Arc::new(vec![previous]), true).unwrap();
                let mut file = super::tests::fixture();
                file.text = "needle\n".repeat(4000);
                file.media.clear();
                let harness = base.with_files(Arc::new(vec![file]), enabled).unwrap();
                let catalog = Arc::new(sovra_core::skills::SkillCatalog::default());
                let harness = harness.with_skill_catalog(catalog.clone()).unwrap();
                let file_skill = catalog.find("read-files").unwrap();
                let harness = if enabled {
                    harness.with_skill(file_skill).unwrap()
                } else {
                    assert!(
                        harness.with_skill(file_skill).is_err(),
                        "a skill cannot enable disabled tools"
                    );
                    harness
                };
                let (tx, rx) = async_channel::unbounded();
                harness
                    .run_with_files("file-test", "needle", 300, &tx)
                    .await;
                let mut answer = String::new();
                let mut completed = false;
                while let Ok(event) = rx.try_recv() {
                    match event {
                        Event::AgentChunk { text, .. } => answer.push_str(&text),
                        Event::AgentCompleted { .. } => completed = true,
                        Event::AgentError { message, .. } => panic!("{message}"),
                        _ => {}
                    }
                }
                assert!(completed);
                assert!(answer.contains(if enabled {
                    "page two"
                } else {
                    "tools disabled"
                }));
                assert_eq!(
                    llm.calls.load(Ordering::SeqCst),
                    if enabled { 3 } else { 1 }
                );
            }
        });
}
