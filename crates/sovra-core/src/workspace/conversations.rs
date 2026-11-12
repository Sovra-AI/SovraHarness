//! Live transcripts move between the visible pane and their owning conversations.
//! Swapping buffers avoids copying a growing transcript on every streamed chunk.

use std::collections::BTreeMap;

use super::{ChatMessage, ChatThread, Event, WorkspaceState};
use crate::{Command, Speaker};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ConversationRun {
    messages: Vec<ChatMessage>,
    queued: Vec<String>,
    generating: bool,
    team_mode: bool,
    speakers: BTreeMap<String, Speaker>,
    awaiting_setup: Option<(String, String)>,
}

impl ConversationRun {
    fn swap(&mut self, workspace: &mut WorkspaceState) {
        std::mem::swap(&mut self.messages, &mut workspace.messages);
        std::mem::swap(&mut self.queued, &mut workspace.queued_prompts);
        std::mem::swap(&mut self.generating, &mut workspace.is_generating);
        std::mem::swap(&mut self.team_mode, &mut workspace.team_mode);
        std::mem::swap(&mut self.speakers, &mut workspace.team_subagents);
        std::mem::swap(
            &mut self.awaiting_setup,
            &mut workspace.prompt_awaiting_setup,
        );
    }

    fn retain(&mut self) -> bool {
        if !self.generating {
            // Completed wording already belongs to the saved history snapshot.
            self.messages.clear();
            self.speakers.clear();
        }
        self.generating
            || !self.queued.is_empty()
            || self.awaiting_setup.is_some()
            || self.team_mode
    }
}

impl WorkspaceState {
    pub(super) fn link_memory_source(
        &mut self,
        saved: &crate::MemorySourceMessage,
        source: &crate::EpisodeId,
    ) {
        if saved.id.is_none()
            && (saved.role == crate::MessageRole::User || saved.in_reply_to.is_none())
        {
            return;
        }
        let matched = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, message)| {
                (message.role == saved.role
                    && message.text == saved.text
                    && saved.id.as_ref().is_none_or(|id| id == &message.id)
                    && saved.in_reply_to.as_ref().is_none_or(|id| {
                        self.messages[..index]
                            .iter()
                            .rev()
                            .find(|message| message.role == crate::MessageRole::User)
                            .is_some_and(|message| &message.id == id)
                    }))
                .then_some(index)
            });
        if let Some(index) = matched {
            self.messages[index]
                .source
                .get_or_insert_with(|| source.clone());
            // Admission metadata does not make an older conversation newly active.
            if let Some(thread) = self
                .threads
                .iter_mut()
                .find(|thread| thread.id == self.session_id)
            {
                thread.messages.clone_from(&self.messages);
            }
        }
    }
    pub(super) fn park_conversation(&mut self) {
        let mut run = ConversationRun::default();
        run.swap(self);
        if run.retain() {
            self.background_conversations
                .insert(self.session_id.clone(), run);
        }
    }

    pub(super) fn focus_conversation(&mut self, thread: &ChatThread) {
        let mut run = self
            .background_conversations
            .remove(&thread.id)
            .unwrap_or_default();
        if run.messages.is_empty() {
            run.messages.clone_from(&thread.messages);
        }
        run.swap(self);
        self.session_id.clone_from(&thread.id);
    }

    /// Fold into the event's existing, non-deleted conversation, keeping the
    /// selected pane unchanged. Unknown sessions never create phantom chats.
    pub fn apply_event(&mut self, event: &Event) -> Option<Command> {
        if let Some(session) = event.session_id()
            && session != self.session_id
        {
            let thread = self
                .threads
                .iter()
                .find(|thread| thread.id == session && thread.is_active())?;
            let mut run = self
                .background_conversations
                .remove(session)
                .unwrap_or_default();
            if run.messages.is_empty() {
                run.messages.clone_from(&thread.messages);
            }
            let visible = std::mem::replace(&mut self.session_id, session.to_owned());
            run.swap(self);
            let next = self
                .fold_event(event)
                .map(|prompt| self.prompt_command(&prompt));
            run.swap(self);
            self.session_id = visible;
            if run.retain() {
                self.background_conversations
                    .insert(session.to_owned(), run);
            }
            return next;
        }
        self.fold_event(event)
            .map(|prompt| self.prompt_command(&prompt))
    }

    /// The command carries its conversation and bounded context, so navigation
    /// between reduction and dispatch cannot redirect a queued follow-up.
    pub fn prompt_command(&self, prompt: &str) -> Command {
        if let Some(prompt) = self.team_prompt(prompt) {
            return Command::Teams(crate::teams::TeamCommand::Run {
                session_id: self.session_id.clone(),
                prompt: prompt.to_owned(),
                context: crate::ConversationContext::before_current_turn(&self.messages),
            });
        }
        let context = Some(crate::ConversationContext::before_current_turn(
            &self.messages,
        ));
        if let Some(message) = self
            .messages
            .iter()
            .rev()
            .find(|message| !message.attachments.is_empty())
        {
            Command::RunWithFiles {
                session_id: self.session_id.clone(),
                prompt: prompt.to_owned(),
                context,
                attachment_ids: message
                    .attachments
                    .iter()
                    .map(|file| file.id.clone())
                    .collect(),
            }
        } else {
            Command::RunPrompt {
                session_id: self.session_id.clone(),
                prompt: prompt.to_owned(),
                context,
            }
        }
    }

    /// Setup completed while this conversation was elsewhere; keep its original request.
    pub fn resume_ready_prompt(&mut self) -> Option<Command> {
        self.take_prompt_awaiting_setup()
            .map(|prompt| self.prompt_command(&prompt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgUiEvent, Timestamp};

    #[test]
    fn durable_reply_receipt_links_the_exact_visible_message_to_its_source() {
        let mut state = WorkspaceState::new_at(1, Timestamp(100));
        let session = state.session_id.clone();
        state.submit_prompt("Give a fixture greeting");
        state.apply_event(&Event::AgentChunk {
            session_id: session.clone(),
            text: "Hello fixture".into(),
        });
        state.apply_event(&Event::Memory(crate::MemoryEvent::SourceSaved {
            session_id: session,
            message: Some(crate::MemorySourceMessage {
                id: None,
                in_reply_to: Some(state.messages[0].id.clone()),
                role: crate::MessageRole::Assistant,
                text: "Hello fixture".into(),
            }),
            receipt: crate::MemorySourceReceipt {
                source_id: crate::EpisodeId("episode:reply".into()),
                version: 1,
                has_processing_job: false,
            },
        }));
        let saved = serde_json::to_value(state.messages.last().unwrap()).unwrap();
        assert_eq!(
            saved.get("source"),
            Some(&serde_json::json!("episode:reply"))
        );
    }

    #[test]
    fn late_receipts_use_exact_turn_identity_without_overwriting_or_reordering_chats() {
        let mut state = WorkspaceState::new_at(1, Timestamp(100));
        let session = state.session_id.clone();
        state.submit_prompt("First question");
        let user = state.messages[0].id.clone();
        state.apply_event(&Event::AgentChunk {
            session_id: session.clone(),
            text: "Same answer".into(),
        });
        state.apply_event(&Event::AgentCompleted {
            session_id: session.clone(),
        });
        let before = state
            .threads
            .iter()
            .find(|thread| thread.id == session)
            .unwrap()
            .updated_at;
        state.start_new_chat();
        state.submit_prompt("Other question");
        let selected = state.session_id.clone();
        for (parent, text, source) in [
            ("wrong", "Same answer", "episode:wrong-turn"),
            (user.as_str(), "Changed answer", "episode:wrong-text"),
            (user.as_str(), "Same answer", "episode:correct"),
            (user.as_str(), "Same answer", "episode:replacement"),
        ] {
            state.apply_event(&Event::Memory(crate::MemoryEvent::SourceSaved {
                session_id: session.clone(),
                message: Some(crate::MemorySourceMessage {
                    id: None,
                    in_reply_to: Some(parent.into()),
                    role: crate::MessageRole::Assistant,
                    text: text.into(),
                }),
                receipt: crate::MemorySourceReceipt {
                    source_id: crate::EpisodeId(source.into()),
                    version: 1,
                    has_processing_job: false,
                },
            }));
        }
        assert_eq!(state.session_id, selected);
        assert!(
            state
                .messages
                .iter()
                .all(|message| message.source.is_none())
        );
        let thread = state
            .threads
            .iter()
            .find(|thread| thread.id == session)
            .unwrap();
        assert_eq!(thread.updated_at, before);
        assert_eq!(
            thread.messages.last().unwrap().source,
            Some(crate::EpisodeId("episode:correct".into()))
        );
    }

    fn team_event(state: &mut WorkspaceState, session: &str, event: AgUiEvent) {
        state.apply_event(&Event::AgUi {
            session_id: session.into(),
            event,
        });
    }

    #[test]
    fn concurrent_team_runs_keep_speakers_and_completion_in_their_own_conversations() {
        let mut state = WorkspaceState::new_at(1, Timestamp(100));
        let first = state.session_id.clone();
        state.submit_prompt("@priya First question");
        team_event(
            &mut state,
            &first,
            AgUiEvent::RunStarted { run_id: "a".into() },
        );
        team_event(
            &mut state,
            &first,
            AgUiEvent::SubagentStarted {
                subagent_run_id: "a-member".into(),
                name: "priya".into(),
                description: "helper".into(),
                speaker: Speaker {
                    member_id: "priya".into(),
                    name: "Priya".into(),
                    icon: String::new(),
                    role: "Researcher".into(),
                },
            },
        );
        let second = state.start_new_chat().id;
        state.submit_prompt("Second question");
        team_event(
            &mut state,
            &second,
            AgUiEvent::RunStarted { run_id: "b".into() },
        );
        team_event(
            &mut state,
            &second,
            AgUiEvent::SubagentStarted {
                subagent_run_id: "b-member".into(),
                name: "kai".into(),
                description: "helper".into(),
                speaker: Speaker {
                    member_id: "kai".into(),
                    name: "Kai".into(),
                    icon: String::new(),
                    role: "Helper".into(),
                },
            },
        );
        team_event(
            &mut state,
            &first,
            AgUiEvent::TextMessageStart {
                message_id: "a-reply".into(),
                subagent_run_id: Some("a-member".into()),
            },
        );
        team_event(
            &mut state,
            &first,
            AgUiEvent::TextMessageContent {
                message_id: "a-reply".into(),
                delta: "First answer, 茶.".into(),
            },
        );
        team_event(
            &mut state,
            &first,
            AgUiEvent::TextMessageEnd {
                message_id: "a-reply".into(),
            },
        );
        team_event(
            &mut state,
            &first,
            AgUiEvent::RunFinished { run_id: "a".into() },
        );
        assert_eq!(state.session_id, second);
        assert_eq!(state.messages.len(), 1);
        assert!(state.is_generating);
        team_event(
            &mut state,
            &second,
            AgUiEvent::TextMessageStart {
                message_id: "b-reply".into(),
                subagent_run_id: Some("b-member".into()),
            },
        );
        team_event(
            &mut state,
            &second,
            AgUiEvent::TextMessageContent {
                message_id: "b-reply".into(),
                delta: "Second answer".into(),
            },
        );
        assert_eq!(state.messages.last().unwrap().author, "Kai");
        assert!(state.select_thread(&first));
        assert!(!state.is_generating);
        let reply = state.messages.last().unwrap();
        assert_eq!(reply.text, "First answer, 茶.");
        assert_eq!(reply.author, "Priya");
        assert_eq!(reply.speaker.as_ref().unwrap().member_id, "priya");
        state.apply_event(&Event::AgentError {
            session_id: second.clone(),
            message: "Second request failed".into(),
        });
        assert!(state.select_thread(&second));
        assert!(!state.is_generating);
        assert!(
            state
                .messages
                .last()
                .unwrap()
                .text
                .contains("Second request failed")
        );
    }

    #[test]
    fn deleting_background_work_discards_its_queue_and_late_events_cannot_revive_it() {
        let mut state = WorkspaceState::new(1);
        let first = state.session_id.clone();
        state.submit_prompt("First");
        state.submit_prompt("Queued");
        let second = state.start_new_chat().id;
        state.submit_prompt("Second");
        assert!(state.delete_thread(&first));
        let deleted = state
            .threads
            .iter()
            .find(|thread| thread.id == first)
            .unwrap()
            .clone();
        for event in [
            Event::AgentChunk {
                session_id: first.clone(),
                text: "late text".into(),
            },
            Event::AgentCompleted {
                session_id: first.clone(),
            },
        ] {
            assert!(state.apply_event(&event).is_none());
        }
        assert_eq!(
            state.threads.iter().find(|thread| thread.id == first),
            Some(&deleted)
        );
        assert_eq!(state.session_id, second);
        assert!(state.is_generating);
        assert!(!state.select_thread(&first));
    }

    #[test]
    fn returning_after_setup_keeps_the_original_unsent_request() {
        let mut state = WorkspaceState::new(1);
        let first = state.session_id.clone();
        state.models_loaded = true;
        state.has_active_llm = false;
        assert!(state.submit_prompt("The original request").is_none());
        state.start_new_chat();
        state.has_active_llm = true;
        assert!(state.resume_ready_prompt().is_none());
        assert!(state.select_thread(&first));
        assert!(
            matches!(state.resume_ready_prompt(), Some(Command::RunPrompt { session_id, prompt, .. }) if session_id == first && prompt == "The original request")
        );
        assert!(state.resume_ready_prompt().is_none());
        assert!(state.is_generating);
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn without_an_ai_a_prompt_still_goes_out_when_the_group_can_answer() {
        let mut state = WorkspaceState::new(1);
        state.models_loaded = true;
        state.has_active_llm = false;
        assert_eq!(
            state.submit_prompt("What's for dinner?"),
            None,
            "waits for setup"
        );

        let mut state = WorkspaceState::new(1);
        state.models_loaded = true;
        state.has_active_llm = false;
        state.group_can_answer = true;
        assert!(
            state.submit_prompt("What's for dinner?").is_some(),
            "a free computer in the group answers instead"
        );
    }

    #[test]
    fn the_work_log_sits_above_the_answer_in_the_same_reply() {
        let mut state = WorkspaceState::new_at(1, Timestamp(100));
        let session = state.session_id.clone();
        state.submit_prompt("Find me a soup recipe");
        let mut work = crate::work::WorkActivity::default();
        let args = serde_json::json!({"query": "soup"});
        work.start(crate::work::StepCall {
            id: "c1",
            helper: "",
            helper_name: "",
            tool: "web_search",
            args: &args,
            asks_helper: None,
            at_ms: 0,
        });
        let snapshot = |work: &crate::work::WorkActivity| AgUiEvent::ActivitySnapshot {
            message_id: "work-1".into(),
            activity: crate::LiveActivity::Work(work.clone()),
        };
        team_event(&mut state, &session, snapshot(&work));
        work.finish_step("c1", true, 900);
        team_event(&mut state, &session, snapshot(&work));
        state.apply_event(&Event::AgentChunk {
            session_id: session.clone(),
            text: "Try leek soup.".into(),
        });
        let replies: Vec<&ChatMessage> = state
            .messages
            .iter()
            .filter(|m| m.role == crate::MessageRole::Assistant)
            .collect();
        assert_eq!(replies.len(), 1, "one reply holds the log and the answer");
        assert_eq!(replies[0].text, "Try leek soup.");
        let logs: Vec<&crate::work::WorkActivity> = replies[0]
            .widgets
            .iter()
            .filter_map(|widget| match widget {
                crate::AgUiWidget::WorkLog(work) => Some(work),
                _ => None,
            })
            .collect();
        assert_eq!(logs.len(), 1, "each snapshot replaces the log in place");
        assert_eq!(logs[0].steps[0].state, crate::work::StepState::Done);
    }
}
