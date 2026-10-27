//! `Llm` decorator that turns a tool call written as text into a real call.
//!
//! ADK normalizes `<tool_call>` markup, but small local models often print a
//! bare JSON call instead, so nothing runs and the user sees JSON. Replies that
//! start like JSON are held back until the stream ends; the pure rule in
//! `sovra_core::parse_text_tool_call` decides whether the whole reply is one
//! call to an offered tool. Everything else streams through untouched.

use std::collections::VecDeque;
use std::sync::Arc;

use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part, SchemaAdapter};
use futures::stream::{self, StreamExt as _};
use sovra_core::{may_be_text_tool_call, split_text_tool_call, text_tool_call_start};

pub(crate) struct TextToolCalls {
    inner: Arc<dyn Llm>,
}

impl TextToolCalls {
    pub(crate) fn wrap(inner: Arc<dyn Llm>) -> Arc<dyn Llm> {
        Arc::new(Self { inner })
    }
}

#[async_trait::async_trait]
impl Llm for TextToolCalls {
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
        let tools: Vec<String> = request.tools.keys().cloned().collect();
        let inner = self.inner.generate_content(request, stream).await?;
        if tools.is_empty() {
            return Ok(inner);
        }
        let state = State::new(inner, tools);
        Ok(Box::pin(stream::unfold(state, |mut state| async move {
            loop {
                if let Some(item) = state.out.pop_front() {
                    return Some((item, state));
                }
                if state.done {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(chunk)) => state.accept(chunk),
                    Some(Err(error)) => {
                        state.flush();
                        state.out.push_back(Err(error));
                    }
                    None => {
                        state.done = true;
                        state.finish();
                        state.flush();
                    }
                }
            }
        })))
    }
}

struct State {
    inner: LlmResponseStream,
    tools: Vec<String>,
    /// A real function call arrived: forward everything untouched.
    passthrough: bool,
    held: Vec<LlmResponse>,
    /// Byte offset in `text` where the held candidate call begins.
    hold_from: Option<usize>,
    /// Text already forwarded; held chunks start after it.
    forwarded: usize,
    text: String,
    out: VecDeque<adk_core::Result<LlmResponse>>,
    done: bool,
}

impl State {
    fn new(inner: LlmResponseStream, tools: Vec<String>) -> Self {
        Self {
            inner,
            tools,
            passthrough: false,
            held: Vec::new(),
            hold_from: None,
            forwarded: 0,
            text: String::new(),
            out: VecDeque::new(),
            done: false,
        }
    }

    fn accept(&mut self, chunk: LlmResponse) {
        if self.passthrough {
            self.out.push_back(Ok(chunk));
            return;
        }
        if chunk_parts(&chunk).any(|part| matches!(part, Part::FunctionCall { .. })) {
            self.flush();
            self.passthrough = true;
            self.out.push_back(Ok(chunk));
            return;
        }
        if let Some(content) = &chunk.content {
            if content_complete(&chunk) {
                // A final chunk that restates the whole reply replaces the deltas.
                self.text = text_of(content);
            } else {
                self.text.push_str(&text_of(content));
            }
        }
        if let Some(start) = self.hold_from {
            self.held.push(chunk);
            if !may_be_text_tool_call(self.text.get(start..).unwrap_or_default()) {
                self.flush();
            }
            return;
        }
        if let Some(start) = text_tool_call_start(&self.text, self.forwarded) {
            self.hold_from = Some(start);
            self.held.push(chunk);
        } else {
            self.out.push_back(Ok(chunk));
            self.forwarded = self.text.len();
        }
    }

    fn flush(&mut self) {
        self.out.extend(self.held.drain(..).map(Ok));
        self.hold_from = None;
        self.forwarded = self.text.len();
    }

    fn finish(&mut self) {
        let Some(hold_from) = self.hold_from else {
            return;
        };
        let offered: Vec<&str> = self.tools.iter().map(String::as_str).collect();
        // The call may start a blank line earlier than the held text; only
        // whitespace may separate it from what was already shown.
        let call = split_text_tool_call(&self.text, &offered).filter(|(lead_in, _)| {
            *lead_in <= hold_from
                && self
                    .text
                    .get((*lead_in).min(self.forwarded)..self.forwarded)
                    .is_some_and(|gap| gap.trim().is_empty())
        });
        let Some((lead_in, call)) = call else {
            self.flush();
            return;
        };
        tracing::info!(tool = %call.name, "recovered a tool call the model wrote as text");
        let mut parts = Vec::with_capacity(2);
        let lead_text = self
            .text
            .get(self.forwarded..lead_in.max(self.forwarded))
            .unwrap_or_default()
            .trim_end();
        if !lead_text.is_empty() {
            parts.push(Part::Text {
                text: lead_text.to_owned(),
            });
        }
        parts.push(Part::FunctionCall {
            name: call.name,
            args: call.args,
            id: None,
            thought_signature: None,
        });
        let mut response = self.held.pop().unwrap_or_default();
        self.held.clear();
        // The held chunks are replaced, so this must not replace earlier text.
        if let Some(serde_json::Value::Object(metadata)) = response.provider_metadata.as_mut() {
            metadata.remove("content_complete");
        }
        response.content = Some(Content {
            role: "model".to_owned(),
            parts,
        });
        response.partial = false;
        response.turn_complete = true;
        self.hold_from = None;
        self.out.push_back(Ok(response));
    }
}

fn chunk_parts(chunk: &LlmResponse) -> impl Iterator<Item = &Part> {
    chunk
        .content
        .iter()
        .flat_map(|content| content.parts.iter())
}

fn content_complete(chunk: &LlmResponse) -> bool {
    !chunk.partial
        && chunk
            .provider_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("content_complete"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

fn text_of(content: &Content) -> String {
    content
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use adk_core::FinishReason;
    use serde_json::json;

    use super::*;

    struct Scripted {
        chunks: Vec<&'static str>,
    }

    #[async_trait::async_trait]
    impl Llm for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let last = self.chunks.len() - 1;
            let chunks: Vec<adk_core::Result<LlmResponse>> = self
                .chunks
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    Ok(LlmResponse {
                        content: Some(Content::new("model").with_text(*text)),
                        partial: index != last,
                        turn_complete: index == last,
                        finish_reason: (index == last).then_some(FinishReason::Stop),
                        ..LlmResponse::default()
                    })
                })
                .collect();
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    fn request(tools: &[&str]) -> LlmRequest {
        let mut request = LlmRequest::new("scripted", Vec::new());
        let mut map = HashMap::new();
        for tool in tools {
            map.insert((*tool).to_owned(), json!({ "name": tool }));
        }
        request.tools = map;
        request
    }

    fn drain(llm: &Arc<dyn Llm>, tools: &[&str]) -> Vec<LlmResponse> {
        futures::executor::block_on(async {
            llm.generate_content(request(tools), true)
                .await
                .expect("stream")
                .map(|item| item.expect("chunk"))
                .collect()
                .await
        })
    }

    #[test]
    fn streamed_json_call_becomes_one_function_call() {
        let llm = TextToolCalls::wrap(Arc::new(Scripted {
            chunks: vec![
                "{\"name\": \"WebSearchAgent\", ",
                "\"arguments\": {\"request\": \"rust site\"}}",
            ],
        }));
        let out = drain(&llm, &["WebSearchAgent"]);
        assert_eq!(out.len(), 1);
        let parts = &out[0].content.as_ref().expect("content").parts;
        assert!(matches!(
            &parts[..],
            [Part::FunctionCall { name, args, .. }]
                if name == "WebSearchAgent" && args == &json!({ "request": "rust site" })
        ));
        assert!(out[0].turn_complete);
    }

    #[test]
    fn lead_in_sentence_streams_and_the_trailing_call_runs() {
        let llm = TextToolCalls::wrap(Arc::new(Scripted {
            chunks: vec![
                "To find it, I'll ask the WebSearchAgent.\n\n",
                "{\"name\": \"WebSearchAgent\", ",
                "\"arguments\": { \"request\": \"latest Rust\" }}",
            ],
        }));
        let out = drain(&llm, &["WebSearchAgent"]);
        assert_eq!(out.len(), 2, "lead-in forwarded, call held: {out:?}");
        let first = &out[0].content.as_ref().expect("lead-in").parts;
        assert!(matches!(&first[..], [Part::Text { text }] if text.starts_with("To find it")));
        let last = &out[1].content.as_ref().expect("call").parts;
        assert!(matches!(
            &last[..],
            [Part::FunctionCall { name, .. }] if name == "WebSearchAgent"
        ));
    }

    #[test]
    fn inline_braces_and_code_blocks_keep_streaming() {
        let llm = TextToolCalls::wrap(Arc::new(Scripted {
            chunks: vec!["Use ", "{x} for sets.\n", "```rust\n", "fn main() {}\n```"],
        }));
        let out = drain(&llm, &["WebSearchAgent"]);
        assert_eq!(out.len(), 4, "nothing may be buffered");
    }

    #[test]
    fn prose_streams_through_unchanged_and_unoffered_calls_stay_text() {
        let llm = TextToolCalls::wrap(Arc::new(Scripted {
            chunks: vec!["Rust ", "is great."],
        }));
        let out = drain(&llm, &["WebSearchAgent"]);
        assert_eq!(out.len(), 2, "prose must not be buffered into one chunk");

        let llm = TextToolCalls::wrap(Arc::new(Scripted {
            chunks: vec!["{\"name\": \"rm_rf\", \"arguments\": {}}"],
        }));
        let out = drain(&llm, &["WebSearchAgent"]);
        let parts = &out[0].content.as_ref().expect("content").parts;
        assert!(matches!(&parts[..], [Part::Text { .. }]));

        // No tools offered: the model's stream is returned as-is.
        let out = drain(&llm, &[]);
        assert_eq!(out.len(), 1);
    }
}
