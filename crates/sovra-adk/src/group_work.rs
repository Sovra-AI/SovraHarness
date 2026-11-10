//! Work handoff through the group (`sovra_core::group::work`), on both sides.
//!
//! - [`GroupLlm`] is the model a computer with no AI of its own uses: it turns the
//!   conversation into a text-only chat job and hands it to a free computer in the
//!   group. Tool declarations never travel; a helper that answers with text is all
//!   a handed-off turn can do.
//! - [`HarnessWork`] answers such a job on this computer's own AI, with no tools, so
//!   another person's request can never reach this computer's tools or files.

use std::sync::Arc;

use adk_core::{Content, FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
use futures::StreamExt as _;
use sovra_core::group::{ChatJob, JobTurn, WorkError, WorkModel, WorkPeer, check_job};

/// Longest answer a helping computer sends back, in bytes.
const MAX_ANSWER_BYTES: usize = 256 * 1024;

fn text_of(content: &Content) -> String {
    content
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The words of a model request as a chat job: system text becomes the
/// instructions, and only text parts of the person's and the AI's turns are kept.
pub fn job_from_request(request: &LlmRequest) -> ChatJob {
    let mut instructions = Vec::new();
    let mut turns = Vec::new();
    for content in &request.contents {
        let text = text_of(content);
        if text.trim().is_empty() {
            continue;
        }
        match content.role.as_str() {
            "system" => instructions.push(text),
            "model" | "assistant" => turns.push(JobTurn {
                from_person: false,
                text,
            }),
            _ => turns.push(JobTurn {
                from_person: true,
                text,
            }),
        }
    }
    ChatJob {
        instructions: instructions.join("\n\n"),
        turns,
    }
    .trimmed()
}

fn unavailable(message: String) -> adk_core::AdkError {
    adk_core::AdkError::new(
        adk_core::ErrorComponent::Model,
        adk_core::ErrorCategory::Unavailable,
        "model.sovra.group_helper",
        message,
    )
}

/// A model that is another computer in the group.
pub struct GroupLlm {
    peer: Arc<dyn WorkPeer>,
}

impl std::fmt::Debug for GroupLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupLlm").finish_non_exhaustive()
    }
}

impl GroupLlm {
    pub fn new(peer: Arc<dyn WorkPeer>) -> Self {
        Self { peer }
    }
}

#[async_trait::async_trait]
impl Llm for GroupLlm {
    fn name(&self) -> &'static str {
        "group-helper"
    }

    async fn generate_content(
        &self,
        request: LlmRequest,
        _stream: bool,
    ) -> adk_core::Result<LlmResponseStream> {
        let job = job_from_request(&request);
        let answer = self.peer.run(job).await.map_err(|error| {
            unavailable(format!(
                "{error}. Connect an AI in Models, or try again when another computer is on."
            ))
        })?;
        let response = LlmResponse {
            content: Some(Content::new("model").with_text(answer.text)),
            finish_reason: Some(FinishReason::Stop),
            ..LlmResponse::default()
        };
        Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
    }
}

/// This computer's own AI answering a group member's chat job, without tools.
pub struct HarnessWork {
    llm: Arc<dyn Llm>,
}

impl std::fmt::Debug for HarnessWork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarnessWork").finish_non_exhaustive()
    }
}

impl HarnessWork {
    pub fn new(llm: Arc<dyn Llm>) -> Self {
        Self { llm }
    }
}

#[async_trait::async_trait]
impl WorkModel for HarnessWork {
    async fn answer(&self, job: ChatJob) -> Result<String, WorkError> {
        check_job(&job)?;
        let mut contents = Vec::new();
        if !job.instructions.trim().is_empty() {
            contents.push(Content::new("system").with_text(job.instructions.clone()));
        }
        for turn in &job.turns {
            let role = if turn.from_person { "user" } else { "model" };
            contents.push(Content::new(role).with_text(turn.text.clone()));
        }
        // No tool declarations: the request carries only words.
        let request = LlmRequest::new(self.llm.name(), contents);
        let mut responses = self
            .llm
            .generate_content(request, false)
            .await
            .map_err(|error| WorkError::Failed(error.to_string()))?;
        let mut answer = String::new();
        while let Some(response) = responses.next().await {
            let response = response.map_err(|error| WorkError::Failed(error.to_string()))?;
            if let Some(content) = response.content {
                for part in content.parts {
                    if let Part::Text { text } = part {
                        if answer.len().saturating_add(text.len()) > MAX_ANSWER_BYTES {
                            return Err(WorkError::Failed("the answer was too long".into()));
                        }
                        answer.push_str(&text);
                    }
                }
            }
        }
        if answer.trim().is_empty() {
            return Err(WorkError::Failed("the AI returned no words".into()));
        }
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Records what it was asked and answers with a fixed reply.
    struct Recording {
        seen: Mutex<Vec<LlmRequest>>,
    }

    #[async_trait::async_trait]
    impl Llm for Recording {
        fn name(&self) -> &'static str {
            "recording"
        }
        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            self.seen.lock().expect("not poisoned").push(request);
            let response = LlmResponse {
                content: Some(Content::new("model").with_text("Pasta tonight.")),
                finish_reason: Some(FinishReason::Stop),
                ..LlmResponse::default()
            };
            Ok(Box::pin(futures::stream::once(async move { Ok(response) })))
        }
    }

    struct Helper;

    #[async_trait::async_trait]
    impl WorkPeer for Helper {
        async fn run(&self, job: ChatJob) -> Result<sovra_core::group::JobAnswer, WorkError> {
            Ok(sovra_core::group::JobAnswer {
                text: format!(
                    "answered: {}",
                    job.turns.last().map_or("", |turn| &turn.text)
                ),
                helper: "Dad".into(),
                computer: "Desktop".into(),
            })
        }
    }

    #[tokio::test]
    async fn a_helper_answers_with_words_only_and_never_sees_tools() {
        let llm = Arc::new(Recording {
            seen: Mutex::new(Vec::new()),
        });
        let work = HarnessWork::new(llm.clone());
        let answer = work
            .answer(ChatJob {
                instructions: "Be brief.".into(),
                turns: vec![JobTurn {
                    from_person: true,
                    text: "What's for dinner?".into(),
                }],
            })
            .await
            .expect("answers");
        assert_eq!(answer, "Pasta tonight.");
        let seen = llm.seen.lock().expect("not poisoned");
        assert!(
            seen[0].tools.is_empty(),
            "no tool reaches the helping computer"
        );
        assert_eq!(seen[0].contents.len(), 2);
    }

    #[tokio::test]
    async fn a_computer_without_ai_hands_the_conversation_to_its_group() {
        let mut request = LlmRequest::new(
            "any",
            vec![
                Content::new("system").with_text("You are Sovra."),
                Content::new("user").with_text("Hi"),
                Content::new("model").with_text("Hello!"),
                Content::new("user").with_text("What's for dinner?"),
            ],
        );
        request
            .tools
            .insert("terminal".into(), serde_json::json!({"name": "terminal"}));
        let job = job_from_request(&request);
        assert_eq!(job.instructions, "You are Sovra.");
        assert_eq!(job.turns.len(), 3);
        assert!(job.turns[2].from_person);

        let llm = GroupLlm::new(Arc::new(Helper));
        let mut responses = llm.generate_content(request, true).await.expect("answers");
        let reply = responses.next().await.expect("one").expect("ok");
        let text = reply
            .content
            .map(|content| text_of(&content))
            .unwrap_or_default();
        assert_eq!(text, "answered: What's for dinner?");
    }
}
