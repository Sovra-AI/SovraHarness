//! `Llm` decorator for the orchestrator: a final answer built on specialists'
//! results keeps their sources (`sovra_core::sources_footer`).
//!
//! Chunks stream through one behind, so the last one can gain the footer when
//! the model's reply turned out to have no links.

use std::sync::Arc;

use adk_core::{Llm, LlmRequest, LlmResponse, LlmResponseStream, Part, SchemaAdapter};
use futures::stream::{self, StreamExt as _};
use sovra_core::sources_footer;

pub(crate) struct SourcesFooter {
    inner: Arc<dyn Llm>,
}

impl SourcesFooter {
    pub(crate) fn wrap(inner: Arc<dyn Llm>) -> Arc<dyn Llm> {
        Arc::new(Self { inner })
    }
}

struct State {
    inner: LlmResponseStream,
    replies: Vec<String>,
    text: String,
    pending: Option<LlmResponse>,
    called: bool,
    done: bool,
}

#[async_trait::async_trait]
impl Llm for SourcesFooter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn schema_adapter(&self) -> &dyn SchemaAdapter {
        self.inner.schema_adapter()
    }

    fn uses_interactions_api(&self) -> bool {
        self.inner.uses_interactions_api()
    }

    async fn generate_content(
        &self,
        request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let replies = specialist_replies(&request);
        let inner = self.inner.generate_content(request, stream).await?;
        if replies.is_empty() {
            return Ok(inner);
        }
        let state = State {
            inner,
            replies,
            text: String::new(),
            pending: None,
            called: false,
            done: false,
        };
        Ok(Box::pin(stream::unfold(state, |mut state| async move {
            loop {
                if state.done {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(chunk)) => {
                        for part in chunk.content.iter().flat_map(|content| &content.parts) {
                            match part {
                                Part::Text { text } => state.text.push_str(text),
                                Part::FunctionCall { .. } => state.called = true,
                                _ => {}
                            }
                        }
                        if let Some(previous) = state.pending.replace(chunk) {
                            return Some((Ok(previous), state));
                        }
                    }
                    Some(Err(error)) => {
                        state.done = true;
                        return Some((Err(error), state));
                    }
                    None => {
                        state.done = true;
                        let mut last = state.pending.take()?;
                        if !state.called
                            && let Some(footer) = sources_footer(
                                &state.text,
                                state.replies.iter().map(String::as_str),
                            )
                        {
                            last.content
                                .get_or_insert_with(|| adk_core::Content::new("model"))
                                .parts
                                .push(Part::Text { text: footer });
                        }
                        return Some((Ok(last), state));
                    }
                }
            }
        })))
    }
}

/// Text answers that specialists returned earlier in this turn.
fn specialist_replies(request: &LlmRequest) -> Vec<String> {
    request
        .contents
        .iter()
        .flat_map(|content| &content.parts)
        .filter_map(|part| match part {
            Part::FunctionResponse {
                function_response, ..
            } => function_response
                .response
                .get("response")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use adk_core::{Content, FunctionResponseData};
    use serde_json::json;

    use super::*;

    struct Says(&'static [&'static str]);

    #[async_trait::async_trait]
    impl Llm for Says {
        fn name(&self) -> &'static str {
            "says"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let chunks: Vec<adk_core::Result<LlmResponse>> = self
                .0
                .iter()
                .map(|text| {
                    Ok(LlmResponse {
                        content: Some(Content::new("model").with_text(*text)),
                        ..LlmResponse::default()
                    })
                })
                .collect();
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    fn after_specialist(reply: &str) -> LlmRequest {
        LlmRequest::new(
            "says",
            vec![
                Content::new("user").with_text("latest rust?"),
                Content {
                    role: "user".to_owned(),
                    parts: vec![Part::FunctionResponse {
                        function_response: FunctionResponseData::new(
                            "WebSearchAgent",
                            json!({ "response": reply }),
                        ),
                        id: None,
                        annotations: None,
                    }],
                },
            ],
        )
    }

    fn text(llm: &Arc<dyn Llm>, request: LlmRequest) -> String {
        futures::executor::block_on(async {
            llm.generate_content(request, true)
                .await
                .expect("stream")
                .map(|chunk| chunk.expect("chunk"))
                .flat_map(|chunk| stream::iter(chunk.content.map(|c| c.parts).unwrap_or_default()))
                .filter_map(|part| async move {
                    match part {
                        Part::Text { text } => Some(text),
                        _ => None,
                    }
                })
                .collect::<String>()
                .await
        })
    }

    #[test]
    fn the_rewritten_answer_keeps_the_specialists_sources() {
        let llm = SourcesFooter::wrap(Arc::new(Says(&["Rust ", "1.99.1 is latest."])));
        let out = text(
            &llm,
            after_specialist("1.99.1 [Rust Blog](https://blog.rust-lang.org/releases)"),
        );
        assert!(out.starts_with("Rust 1.99.1 is latest."), "{out}");
        assert!(
            out.ends_with("Sources:\n- [Rust Blog](https://blog.rust-lang.org/releases)"),
            "{out}"
        );
    }

    #[test]
    fn answers_with_links_and_turns_without_specialists_are_untouched() {
        let linked = SourcesFooter::wrap(Arc::new(Says(&["See https://rust-lang.org"])));
        let out = text(&linked, after_specialist("[x](https://blog.rust-lang.org)"));
        assert_eq!(out, "See https://rust-lang.org");

        let plain = SourcesFooter::wrap(Arc::new(Says(&["Hello"])));
        let out = text(
            &plain,
            LlmRequest::new("says", vec![Content::new("user").with_text("hi")]),
        );
        assert_eq!(out, "Hello");
    }
}
