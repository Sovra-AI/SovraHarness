//! `Llm` decorator: a call the turn requires is made even when the model skips it.
//!
//! ADK-Rust has no `tool_choice`. Small models answer "search the web for…"
//! from memory, at the orchestrator (no delegation) and in the specialist (no
//! search). On the turn's first model call, if a [`FirstCall`] rule applies and
//! the reply calls no tool, the reply is replaced by the required call. Later
//! calls (after a tool result) are never touched, so the model still writes the
//! answer from real results.

use std::sync::Arc;

use adk_core::{
    Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part, SchemaAdapter,
};
use futures::stream::{self, StreamExt as _};
use serde_json::json;
use sovra_core::required_delegate;

/// What a turn's first model call must do when the model itself does not.
#[derive(Clone)]
pub(crate) enum FirstCall {
    /// Orchestrator: consult the specialist `required_delegate` names, if any.
    Delegate { procedure_requests: Option<String> },
    /// Equipped specialist: call its capability's tool before answering.
    Tool {
        name: &'static str,
        args: fn(&str) -> serde_json::Value,
    },
}

impl FirstCall {
    fn decide(&self, user_text: &str, offered: &[&str]) -> Option<(String, serde_json::Value)> {
        match self {
            Self::Delegate { procedure_requests } => {
                required_delegate(user_text, offered, procedure_requests.as_deref())
                    .map(|target| (target.to_owned(), json!({ "request": user_text })))
            }
            Self::Tool { name, args } => offered
                .contains(name)
                .then(|| ((*name).to_owned(), args(user_text))),
        }
    }
}

pub(crate) struct RequiredFirstCall {
    inner: Arc<dyn Llm>,
    rule: FirstCall,
}

impl RequiredFirstCall {
    pub(crate) fn wrap(inner: Arc<dyn Llm>, rule: FirstCall) -> Arc<dyn Llm> {
        Arc::new(Self { inner, rule })
    }
}

#[async_trait::async_trait]
impl Llm for RequiredFirstCall {
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
        let Some(user_text) = opening_user_text(&request) else {
            return self.inner.generate_content(request, stream).await;
        };
        let offered: Vec<&str> = request.tools.keys().map(String::as_str).collect();
        let Some((target, args)) = self.rule.decide(&user_text, &offered) else {
            return self.inner.generate_content(request, stream).await;
        };
        // The reply may be replaced, so it is gathered before anything is shown.
        let chunks: Vec<adk_core::Result<LlmResponse>> = self
            .inner
            .generate_content(request, stream)
            .await?
            .collect()
            .await;
        let called = chunks.iter().flatten().any(|chunk| {
            chunk
                .content
                .iter()
                .flat_map(|content| content.parts.iter())
                .any(|part| matches!(part, Part::FunctionCall { .. }))
        });
        if called || chunks.iter().any(Result::is_err) {
            return Ok(Box::pin(stream::iter(chunks)));
        }
        tracing::info!(tool = %target, "model skipped a required call; making it");
        let call = LlmResponse {
            content: Some(Content {
                role: "model".to_owned(),
                parts: vec![Part::FunctionCall {
                    name: target,
                    args,
                    id: None,
                    thought_signature: None,
                }],
            }),
            finish_reason: Some(FinishReason::Stop),
            partial: false,
            turn_complete: true,
            ..LlmResponse::default()
        };
        Ok(Box::pin(stream::iter([Ok(call)])))
    }
}

/// The user's text when this is the turn's first model call: the last content
/// is the user's message, not a tool result coming back.
fn opening_user_text(request: &LlmRequest) -> Option<String> {
    let last = request.contents.last()?;
    if last.role != "user"
        || last
            .parts
            .iter()
            .any(|part| matches!(part, Part::FunctionResponse { .. }))
    {
        return None;
    }
    let text: String = last
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Always answers from memory, the failure seen live with a 7B model.
    struct Stubborn {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Llm for Stubborn {
        fn name(&self) -> &'static str {
            "stubborn"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = LlmResponse {
                content: Some(Content::new("model").with_text("From memory: rust-lang.org")),
                turn_complete: true,
                ..LlmResponse::default()
            };
            Ok(Box::pin(stream::iter([Ok(response)])))
        }
    }

    fn request(contents: Vec<Content>) -> LlmRequest {
        let mut request = LlmRequest::new("stubborn", contents);
        request.tools = HashMap::from([
            ("WebSearchAgent".to_owned(), json!({})),
            ("CodingAgent".to_owned(), json!({})),
        ]);
        request
    }

    fn parts(llm: &Arc<dyn Llm>, request: LlmRequest) -> Vec<Part> {
        futures::executor::block_on(async {
            llm.generate_content(request, true)
                .await
                .expect("stream")
                .map(|item| item.expect("chunk"))
                .flat_map(|chunk| stream::iter(chunk.content.map(|c| c.parts).unwrap_or_default()))
                .collect()
                .await
        })
    }

    #[test]
    fn explicit_search_is_delegated_even_when_the_model_answers_from_memory() {
        let llm = RequiredFirstCall::wrap(
            Arc::new(Stubborn {
                calls: AtomicUsize::new(0),
            }),
            FirstCall::Delegate {
                procedure_requests: None,
            },
        );
        let user = Content::new("user").with_text("Search the web for the Rust site");
        let out = parts(&llm, request(vec![user]));
        assert!(matches!(
            &out[..],
            [Part::FunctionCall { name, args, .. }]
                if name == "WebSearchAgent" && args["request"] == "Search the web for the Rust site"
        ));
    }

    #[test]
    fn an_equipped_specialist_searches_even_when_the_model_answers_from_memory() {
        let llm = RequiredFirstCall::wrap(
            Arc::new(Stubborn {
                calls: AtomicUsize::new(0),
            }),
            FirstCall::Tool {
                name: "web_search",
                args: |request| json!({ "queries": [request] }),
            },
        );
        let mut asked = request(vec![
            Content::new("user").with_text("Official Rust website"),
        ]);
        asked.tools.insert("web_search".to_owned(), json!({}));
        assert!(matches!(
            &parts(&llm, asked)[..],
            [Part::FunctionCall { name, args, .. }]
                if name == "web_search" && args["queries"][0] == "Official Rust website"
        ));
        // Without the tool on offer the rule does nothing.
        let plain = request(vec![
            Content::new("user").with_text("Official Rust website"),
        ]);
        assert!(matches!(&parts(&llm, plain)[..], [Part::Text { .. }]));
    }

    #[test]
    fn implicit_questions_and_follow_up_calls_are_left_to_the_model() {
        let llm = RequiredFirstCall::wrap(
            Arc::new(Stubborn {
                calls: AtomicUsize::new(0),
            }),
            FirstCall::Delegate {
                procedure_requests: None,
            },
        );
        let user = Content::new("user").with_text("hello");
        assert!(matches!(
            &parts(&llm, request(vec![user]))[..],
            [Part::Text { .. }]
        ));

        // After the delegate's result comes back, the orchestrator writes the answer.
        let asked = Content::new("user").with_text("Search the web for the Rust site");
        let result = Content {
            role: "user".to_owned(),
            parts: vec![Part::FunctionResponse {
                function_response: adk_core::FunctionResponseData::new(
                    "WebSearchAgent",
                    json!({ "response": "rust-lang.org" }),
                ),
                id: None,
                annotations: None,
            }],
        };
        assert!(matches!(
            &parts(&llm, request(vec![asked, result]))[..],
            [Part::Text { .. }]
        ));
    }
}
