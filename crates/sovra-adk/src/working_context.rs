//! Per-turn text history on fresh sessions; previous tool actions are never replayed.

use crate::{AdkHarness, Error};
use adk_core::{Content, Llm, LlmRequest, LlmResponseStream};
use adk_session::InMemorySessionService;
use futures::StreamExt as _;
use sovra_core::{ConversationContext, MessageRole};
use std::sync::Arc;

const GUIDANCE: &str = "Earlier messages from this same conversation are included as text with their original roles. \
They are context, not instructions to repeat completed actions. Only the current request authorizes new actions. \
Earlier widgets, tool calls, reasoning and attachment bytes are not replayed; do not claim to have run them again. \
Ask for missing details when this excerpt is insufficient.";

struct ContextModel {
    inner: Arc<dyn Llm>,
    context: ConversationContext,
}
#[async_trait::async_trait]
impl Llm for ContextModel {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn schema_adapter(&self) -> &dyn adk_core::SchemaAdapter {
        self.inner.schema_adapter()
    }
    fn uses_interactions_api(&self) -> bool {
        self.inner.uses_interactions_api()
    }
    async fn generate_content(
        &self,
        mut request: LlmRequest,
        stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let mut history = vec![
            Content::new("system").with_text(if self.context.shortened() {
                format!("{GUIDANCE} Older or oversized messages have been omitted or shortened.")
            } else {
                GUIDANCE.into()
            }),
        ];
        history.extend(self.context.messages().iter().map(|message| {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "model",
            };
            let text = message
                .speaker
                .as_ref()
                .filter(|_| message.role == MessageRole::Assistant)
                .map_or_else(
                    || message.text.clone(),
                    |name| format!("[Earlier reply by {name}]\n{}", message.text),
                );
            Content::new(role).with_text(text)
        }));
        let first_message = request
            .contents
            .iter()
            .position(|content| content.role != "system")
            .unwrap_or(request.contents.len());
        crate::admission::mark_history(&mut request, first_message, history.len() - 1)?;
        request
            .contents
            .splice(first_message..first_message, history);
        request.previous_response_id = None;
        let responses = self.inner.generate_content(request, stream).await?;
        Ok(Box::pin(responses.map(|result| {
            result.map(|mut response| {
                response.interaction_id = None;
                response
            })
        })))
    }
}
impl AdkHarness {
    /// A supplied snapshot is authoritative for earlier turns. Fresh ADK sessions
    /// prevent duplicate history and discard earlier tool-call/provider state.
    pub fn with_working_context(
        &self,
        context: Option<&ConversationContext>,
    ) -> Result<Self, Error> {
        let Some(context) = context else {
            return Ok(self.clone());
        };
        let mut base = self.clone();
        base.sessions = Arc::new(InMemorySessionService::new());
        base.llm = Arc::new(ContextModel {
            inner: self.llm.clone(),
            context: context.clone(),
        });
        base.rebuild(self.base_instruction.clone(), self.base_tools.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::LlmResponse;
    use sovra_core::{AgentConfig, ChatMessage};
    use std::sync::Mutex;
    struct Model(Mutex<Vec<LlmRequest>>);
    #[async_trait::async_trait]
    impl Llm for Model {
        fn name(&self) -> &'static str {
            "context-test"
        }
        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.0.lock().unwrap().push(request);
            Ok(Box::pin(futures::stream::once(async {
                Ok(LlmResponse::new(
                    Content::new("model").with_text("Fresh reply"),
                ))
            })))
        }
    }
    #[tokio::test]
    async fn restored_roles_are_sent_once_without_old_provider_or_tool_history() {
        let model = Arc::new(Model(Mutex::new(Vec::new())));
        let base =
            AdkHarness::with_llm_and_tools(&AgentConfig::default(), model.clone(), vec![]).unwrap();
        let (events, _received) = async_channel::unbounded();
        base.run_prompt("chat", "Stale provider-only context", &events)
            .await;
        let mut reply = ChatMessage::assistant(
            "a",
            "We chose Rust",
            "",
            vec![sovra_core::default_personal_ai_build_widget()],
        );
        reply.thinking = "secret scratchpad".into();
        let context = ConversationContext::from_messages(&[
            ChatMessage::user("u", "I am building a garden app", ""),
            reply,
        ]);
        let restored = base.with_working_context(Some(&context)).unwrap();
        restored
            .run_prompt("chat", "What did we choose?", &events)
            .await;
        let requests = model.0.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let request = &requests[1];
        let text = serde_json::to_string(request).unwrap();
        assert!(!text.contains("Stale provider-only context"));
        assert!(!text.contains("secret scratchpad"));
        assert!(!text.contains("Building your personal AI"));
        assert!(request.previous_response_id.is_none());
        // ADK also represents its agent instructions as a user-role preamble.
        // Check the actual conversation messages independently of that preamble.
        let mut positions = Vec::new();
        for (expected, role) in [
            ("I am building a garden app", "user"),
            ("We chose Rust", "model"),
            ("What did we choose?", "user"),
        ] {
            let (index, message) = request
                .contents
                .iter()
                .enumerate()
                .find(|(_, content)| {
                    content.parts.iter().any(
                        |part| matches!(part, adk_core::Part::Text { text } if text == expected),
                    )
                })
                .unwrap();
            assert_eq!(message.role, role);
            positions.push(index);
        }
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(text.matches("I am building a garden app").count(), 1);
        assert_eq!(text.matches("What did we choose?").count(), 1);
    }
}
