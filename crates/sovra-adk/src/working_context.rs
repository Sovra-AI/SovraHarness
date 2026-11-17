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
        let at = current_turn_start(&request.contents);
        crate::admission::mark_history(&mut request, at, history.len() - 1)?;
        request.contents.splice(at..at, history);
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
/// Where this turn's own conversation begins: its user message, after the agent
/// instructions ADK sends ahead of it as user-role text, and before any tool
/// calls the turn has made. Earlier messages go here. Placed in front of the
/// instructions instead, the model read the instructions as a fresh start and
/// told the user it could not see the chat's history.
fn current_turn_start(contents: &[Content]) -> usize {
    let first_message = contents
        .iter()
        .position(|content| content.role != "system")
        .unwrap_or(contents.len());
    let first_reply = contents[first_message..]
        .iter()
        .position(|content| {
            content.role != "user"
                || content
                    .parts
                    .iter()
                    .any(|part| matches!(part, adk_core::Part::FunctionResponse { .. }))
        })
        .map_or(contents.len(), |offset| first_message + offset);
    first_reply.saturating_sub(1).max(first_message)
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

    fn position_of(request: &LlmRequest, needle: &str) -> usize {
        request
            .contents
            .iter()
            .position(|content| {
                content.parts.iter().any(
                    |part| matches!(part, adk_core::Part::Text { text } if text.contains(needle)),
                )
            })
            .unwrap_or_else(|| panic!("{needle} missing from {:?}", request.contents))
    }

    /// Regression (seen live): the earlier chat was placed in front of the
    /// agent instructions, which ADK sends as user-role text. The model read
    /// the instructions as a fresh start and answered "I don't have access to
    /// the history of your previous messages in this chat session".
    #[tokio::test]
    async fn earlier_messages_follow_the_instructions_and_lead_into_the_new_message() {
        let model = Arc::new(Model(Mutex::new(Vec::new())));
        let base = AdkHarness::with_llm_and_tools(
            &AgentConfig {
                instruction: "INSTRUCTION-SENTINEL: you are Sovra.".into(),
                ..AgentConfig::default()
            },
            model.clone(),
            vec![],
        )
        .unwrap();
        let context = ConversationContext::from_messages(&[
            ChatMessage::user("u", "I want to incorporate an AI company in the UAE", ""),
            ChatMessage::assistant("a", "Dubai Internet City suits it.", "", vec![]),
        ]);
        let (events, _received) = async_channel::unbounded();
        base.with_working_context(Some(&context))
            .unwrap()
            .run_prompt(
                "chat",
                "Quote the very first message I sent in this chat.",
                &events,
            )
            .await;
        let requests = model.0.lock().unwrap();
        let request = &requests[0];
        let instruction = position_of(request, "INSTRUCTION-SENTINEL");
        let guidance = position_of(request, "Earlier messages from this same conversation");
        let first = position_of(request, "I want to incorporate an AI company in the UAE");
        let reply = position_of(request, "Dubai Internet City suits it.");
        let current = position_of(request, "Quote the very first message");
        assert!(
            instruction < guidance && guidance < first && first < reply && reply < current,
            "instructions, then the earlier chat, then the new message: {:?}",
            [instruction, guidance, first, reply, current]
        );
        assert_eq!(current, request.contents.len() - 1);
    }

    #[test]
    fn a_tool_step_keeps_the_earlier_chat_before_the_turns_own_message() {
        let fn_response = Content {
            role: "user".into(),
            parts: vec![adk_core::Part::FunctionResponse {
                function_response: adk_core::FunctionResponseData::new(
                    "tool",
                    serde_json::json!({}),
                ),
                id: None,
                annotations: None,
            }],
        };
        let contents = vec![
            Content::new("system").with_text("notes"),
            Content::new("user").with_text("instructions"),
            Content::new("user").with_text("current question"),
            Content::new("model").with_text("calling a tool"),
            fn_response,
        ];
        assert_eq!(current_turn_start(&contents), 2);
        assert_eq!(current_turn_start(&contents[..3]), 2);
        assert_eq!(
            current_turn_start(&[Content::new("user").with_text("only message")]),
            0
        );
        assert_eq!(current_turn_start(&[]), 0);
    }
}
