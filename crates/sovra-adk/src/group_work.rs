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
use std::sync::atomic::{AtomicBool, Ordering};

use sovra_core::group::{
    ChatJob, DeviceId, JobTurn, WorkError, WorkModel, WorkPeer, WorkTicket, check_job,
};

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

/// Following one handed-off answer: where it is, and whether to stop it if the
/// chat stops reading (the stream is dropped, e.g. the person pressed Stop).
struct Following {
    peer: Arc<dyn WorkPeer>,
    job: ChatJob,
    ticket: WorkTicket,
    offset: usize,
    avoided: Vec<DeviceId>,
    finished: bool,
}

impl Drop for Following {
    fn drop(&mut self) {
        if !self.finished {
            self.peer.cancel(&self.ticket);
        }
    }
}

fn chunk(text: String, last: bool) -> LlmResponse {
    LlmResponse {
        content: (!text.is_empty()).then(|| Content::new("model").with_text(text)),
        partial: !last,
        turn_complete: last,
        finish_reason: last.then_some(FinishReason::Stop),
        ..LlmResponse::default()
    }
}

/// The next piece of the answer, moving to another free computer if the helper
/// disappears before it said anything (nothing is repeated: no tools ran).
async fn next_piece(
    mut following: Following,
) -> Option<(adk_core::Result<LlmResponse>, Following)> {
    if following.finished {
        return None;
    }
    loop {
        match following
            .peer
            .next(&following.ticket, following.offset)
            .await
        {
            Ok(progress) => {
                following.offset += progress.text.len();
                if let Some(reason) = progress.failed {
                    following.finished = true;
                    return Some((
                        Err(unavailable(format!(
                            "The helping computer stopped: {reason}"
                        ))),
                        following,
                    ));
                }
                if progress.done {
                    following.finished = true;
                    return Some((Ok(chunk(progress.text, true)), following));
                }
                if !progress.text.is_empty() {
                    return Some((Ok(chunk(progress.text, false)), following));
                }
            }
            Err(error) if following.offset == 0 && following.avoided.len() < 3 => {
                following.avoided.push(following.ticket.helper);
                if let Ok(ticket) = following
                    .peer
                    .start(following.job.clone(), &following.avoided)
                    .await
                {
                    following.ticket = ticket;
                } else {
                    following.finished = true;
                    return Some((
                        Err(unavailable(format!(
                            "{error}. Connect an AI in Models, or try again when another computer is on."
                        ))),
                        following,
                    ));
                }
            }
            Err(error) => {
                following.finished = true;
                return Some((
                    Err(unavailable(format!(
                        "The helping computer went away mid-answer: {error}"
                    ))),
                    following,
                ));
            }
        }
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
        let ticket = self.peer.start(job.clone(), &[]).await.map_err(|error| {
            unavailable(format!(
                "{error}. Connect an AI in Models, or try again when another computer is on."
            ))
        })?;
        let following = Following {
            peer: self.peer.clone(),
            job,
            ticket,
            offset: 0,
            avoided: Vec::new(),
            finished: false,
        };
        Ok(Box::pin(futures::stream::unfold(following, next_piece)))
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
    async fn answer(
        &self,
        job: ChatJob,
        out: &(dyn for<'a> Fn(&'a str) + Send + Sync),
        cancelled: &AtomicBool,
    ) -> Result<(), WorkError> {
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
            .generate_content(request, true)
            .await
            .map_err(|error| WorkError::Failed(error.to_string()))?;
        let mut written = 0usize;
        while let Some(response) = responses.next().await {
            // Dropping the stream here stops the model's generation.
            if cancelled.load(Ordering::SeqCst) {
                return Ok(());
            }
            let response = response.map_err(|error| WorkError::Failed(error.to_string()))?;
            if let Some(content) = response.content {
                for part in content.parts {
                    if let Part::Text { text } = part {
                        written = written.saturating_add(text.len());
                        if written > MAX_ANSWER_BYTES {
                            return Err(WorkError::Failed("the answer was too long".into()));
                        }
                        out(&text);
                    }
                }
            }
        }
        if written == 0 {
            return Err(WorkError::Failed("the AI returned no words".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::group::WorkProgress;
    use std::sync::Mutex;

    /// Records what it was asked and streams a reply in two pieces.
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
            Ok(Box::pin(futures::stream::iter([
                Ok(chunk("Pasta ".into(), false)),
                Ok(chunk("tonight.".into(), true)),
            ])))
        }
    }

    fn dinner() -> ChatJob {
        ChatJob {
            instructions: "Be brief.".into(),
            turns: vec![JobTurn {
                from_person: true,
                text: "What's for dinner?".into(),
            }],
        }
    }

    #[tokio::test]
    async fn a_helper_streams_words_only_and_never_sees_tools() {
        let llm = Arc::new(Recording {
            seen: Mutex::new(Vec::new()),
        });
        let work = HarnessWork::new(llm.clone());
        let pieces = Mutex::new(Vec::new());
        let out = |text: &str| pieces.lock().expect("not poisoned").push(text.to_owned());
        work.answer(dinner(), &out, &AtomicBool::new(false))
            .await
            .expect("answers");
        assert_eq!(
            *pieces.lock().expect("not poisoned"),
            vec!["Pasta ", "tonight."]
        );
        assert!(
            llm.seen.lock().expect("not poisoned")[0].tools.is_empty(),
            "no tool reaches the helping computer"
        );

        // Cancelled before the first piece: nothing more is written.
        let pieces = Mutex::new(Vec::new());
        let out = |text: &str| pieces.lock().expect("not poisoned").push(text.to_owned());
        work.answer(dinner(), &out, &AtomicBool::new(true))
            .await
            .expect("stops quietly");
        assert!(pieces.lock().expect("not poisoned").is_empty());
    }

    /// A group whose first helper vanishes and whose second streams an answer.
    struct Group {
        started: Mutex<Vec<DeviceId>>,
        cancelled: Mutex<Vec<DeviceId>>,
        pieces: Mutex<Vec<WorkProgress>>,
    }

    fn ticket(n: u8) -> WorkTicket {
        WorkTicket {
            id: sovra_core::group::JobId([n; 16]),
            helper: DeviceId([n; 32]),
            helper_name: format!("Helper {n}"),
            computer: "Desktop".into(),
        }
    }

    #[async_trait::async_trait]
    impl WorkPeer for Group {
        async fn start(&self, _job: ChatJob, avoid: &[DeviceId]) -> Result<WorkTicket, WorkError> {
            let n = if avoid.is_empty() { 1 } else { 2 };
            self.started
                .lock()
                .expect("not poisoned")
                .push(DeviceId([n; 32]));
            Ok(ticket(n))
        }
        async fn next(&self, ticket: &WorkTicket, _from: usize) -> Result<WorkProgress, WorkError> {
            if ticket.helper == DeviceId([1; 32]) {
                return Err(WorkError::Failed("unreachable".into()));
            }
            let mut pieces = self.pieces.lock().expect("not poisoned");
            Ok(if pieces.is_empty() {
                WorkProgress::default()
            } else {
                pieces.remove(0)
            })
        }
        fn cancel(&self, ticket: &WorkTicket) {
            self.cancelled
                .lock()
                .expect("not poisoned")
                .push(ticket.helper);
        }
    }

    fn group(pieces: Vec<WorkProgress>) -> Arc<Group> {
        Arc::new(Group {
            started: Mutex::new(Vec::new()),
            cancelled: Mutex::new(Vec::new()),
            pieces: Mutex::new(pieces),
        })
    }

    fn piece(text: &str, done: bool) -> WorkProgress {
        WorkProgress {
            text: text.into(),
            done,
            failed: None,
        }
    }

    #[tokio::test]
    async fn an_answer_streams_from_the_group_and_moves_on_if_a_helper_vanishes() {
        let request = LlmRequest::new(
            "any",
            vec![
                Content::new("system").with_text("You are Sovra."),
                Content::new("user").with_text("What's for dinner?"),
            ],
        );
        let peer = group(vec![piece("Pasta ", false), piece("tonight.", true)]);
        let llm = GroupLlm::new(peer.clone());
        let responses: Vec<LlmResponse> = llm
            .generate_content(request, true)
            .await
            .expect("started")
            .map(|response| response.expect("ok"))
            .collect()
            .await;
        let texts: Vec<String> = responses
            .iter()
            .map(|response| response.content.as_ref().map(text_of).unwrap_or_default())
            .collect();
        assert_eq!(texts, vec!["Pasta ", "tonight."]);
        assert!(responses[0].partial && !responses[1].partial && responses[1].turn_complete);
        assert_eq!(
            *peer.started.lock().expect("not poisoned"),
            vec![DeviceId([1; 32]), DeviceId([2; 32])],
            "the vanished helper is skipped"
        );
        assert!(
            peer.cancelled.lock().expect("not poisoned").is_empty(),
            "a finished answer is not cancelled"
        );
    }

    #[tokio::test]
    async fn stopping_the_chat_cancels_the_helpers_job() {
        let request = LlmRequest::new("any", vec![Content::new("user").with_text("Long story?")]);
        let peer = group(vec![piece("Once upon ", false), piece("a time", false)]);
        let llm = GroupLlm::new(peer.clone());
        let mut responses = llm.generate_content(request, true).await.expect("started");
        let first = responses.next().await.expect("a piece").expect("ok");
        assert!(first.partial);
        drop(responses);
        assert_eq!(
            *peer.cancelled.lock().expect("not poisoned"),
            vec![DeviceId([2; 32])]
        );
    }

    #[test]
    fn a_requests_words_become_a_job_without_its_tools() {
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
    }
}
