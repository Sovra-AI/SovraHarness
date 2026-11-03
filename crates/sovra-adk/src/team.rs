//! Runs a team turn: members answer in rounds, a Conductor moderates, and every
//! step streams to the UI as AG-UI events (`RUN_STARTED`, `ACTIVITY_SNAPSHOT`,
//! `SUBAGENT_STARTED`, `TEXT_MESSAGE_*`, `SUBAGENT_FINISHED`, `RUN_FINISHED`).
//! The rules — who speaks, passes, verdicts — are pure in `sovra_core::teams`.

use std::collections::HashMap;
use std::sync::Arc;

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, EventTextDeltas};
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use async_channel::Sender;
use futures::StreamExt as _;
use sovra_core::teams::{
    self, MAX_REPLIES_PER_TURN, MAX_ROUNDS, SpeakerState, TeamActivity, TeamMember, TranscriptLine,
    Verdict, VisibleStream,
};
use sovra_core::{AgUiEvent, Event, LiveActivity, Speaker, SubagentOutcome};

use crate::{AdkHarness, DEFAULT_USER_ID, extract_output};

/// One team turn, as the runtime hands it over.
#[derive(Debug, Clone)]
pub struct TeamTurn {
    pub session_id: String,
    /// Unique per turn; prefixes every AG-UI id so messages never collide.
    pub run_id: String,
    pub prompt: String,
    pub history: Vec<TranscriptLine>,
    pub members: Vec<TeamMember>,
}

/// How one member's reply ended.
enum Reply {
    Spoke(String),
    Passed,
    Failed,
}

struct TeamRunState<'a> {
    harness: &'a AdkHarness,
    turn: &'a TeamTurn,
    events: &'a Sender<Event>,
    activity: TeamActivity,
    history: Vec<TranscriptLine>,
    replies: usize,
    activity_id: String,
}

impl AdkHarness {
    /// Streams a whole team turn. Returns when the run finished or the UI went away.
    pub async fn run_team(&self, turn: &TeamTurn, events: &Sender<Event>) {
        let speakers = teams::first_round_speakers(&turn.prompt, &turn.members);
        let conductor = teams::conductor_for(&turn.members, &speakers).cloned();
        // Older callers included the current user line in history. Keep it only
        // in the explicit current-request section, where history limits cannot clip it.
        let mut history = turn.history.clone();
        if history.last().is_some_and(|line| {
            line.speaker == "User"
                && (line.text.trim() == turn.prompt.trim()
                    || teams::strip_team_command(&line.text) == Some(turn.prompt.trim()))
        }) {
            history.pop();
        }
        let mut state = TeamRunState {
            harness: self,
            turn,
            events,
            activity: TeamActivity::new(1, &turn.members, &speakers),
            history,
            replies: 0,
            activity_id: format!("{}-team", turn.run_id),
        };
        if let Some(lead) = &conductor {
            state
                .activity
                .queue(&turn.members, std::slice::from_ref(&lead.id));
        }
        if !state
            .send(AgUiEvent::RunStarted {
                run_id: turn.run_id.clone(),
            })
            .await
            || !state.snapshot().await
        {
            return;
        }
        if speakers.is_empty() {
            state
                .send(AgUiEvent::RunError {
                    message: "Your team has nobody switched on. Open Team to add a helper.".into(),
                })
                .await;
            return;
        }
        let moderated = conductor.is_some();
        let mut speakers = speakers;
        let mut spoke_any = false;
        let mut closing = "Your team is done";
        for round in 1..=MAX_ROUNDS {
            state.activity.round = round;
            for id in &speakers {
                let Some(member) = turn.members.iter().find(|m| m.id == *id) else {
                    continue;
                };
                match state.speak(member, round, moderated).await {
                    None => return,
                    Some(Reply::Spoke(_)) => spoke_any = true,
                    Some(Reply::Passed | Reply::Failed) => {}
                }
            }
            let Some(lead) = &conductor else {
                break;
            };
            let Some(reply) = state.speak(lead, round, true).await else {
                return;
            };
            let decision = match reply {
                Reply::Spoke(text) => {
                    spoke_any = true;
                    teams::parse_conductor_reply(&text).1
                }
                Reply::Passed | Reply::Failed => break,
            };
            match decision.verdict {
                Verdict::Continue if round < MAX_ROUNDS && state.replies < MAX_REPLIES_PER_TURN => {
                    speakers = teams::next_round_speakers(&decision, &turn.members, &speakers);
                    state.activity.queue(&turn.members, &speakers);
                    state
                        .activity
                        .queue(&turn.members, std::slice::from_ref(&lead.id));
                }
                Verdict::AskUser => {
                    closing = "Your team has a question for you";
                    break;
                }
                Verdict::Continue | Verdict::YieldToUser => break,
            }
        }
        if !spoke_any {
            state
                .send(AgUiEvent::RunError {
                    message: "Your team couldn't answer. Check your AI connection in Models and try again.".into(),
                })
                .await;
            return;
        }
        state.activity.finish(closing);
        if state.snapshot().await {
            state
                .send(AgUiEvent::RunFinished {
                    run_id: turn.run_id.clone(),
                })
                .await;
        }
    }
}

impl TeamRunState<'_> {
    async fn send(&self, event: AgUiEvent) -> bool {
        self.events
            .send(Event::AgUi {
                session_id: self.turn.session_id.clone(),
                event,
            })
            .await
            .is_ok()
    }

    async fn snapshot(&self) -> bool {
        self.send(AgUiEvent::ActivitySnapshot {
            message_id: self.activity_id.clone(),
            activity: LiveActivity::Team(self.activity.clone()),
        })
        .await
    }

    /// One member's invocation as an AG-UI subagent. `None` means the UI is gone.
    async fn speak(&mut self, member: &TeamMember, round: u8, moderated: bool) -> Option<Reply> {
        if self.replies >= MAX_REPLIES_PER_TURN {
            return Some(Reply::Passed);
        }
        self.replies += 1;
        let invocation = format!("{}-{}", self.turn.run_id, self.replies);
        let message_id = format!("{invocation}-reply");
        self.activity.set(&member.id, SpeakerState::Speaking);
        let started = self.snapshot().await
            && self
                .send(AgUiEvent::SubagentStarted {
                    subagent_run_id: invocation.clone(),
                    name: member.id.clone(),
                    description: member.role.clone(),
                    speaker: Speaker {
                        member_id: member.id.clone(),
                        name: member.name.clone(),
                        icon: member.icon.clone(),
                        role: member.role.clone(),
                    },
                })
                .await
            && self
                .send(AgUiEvent::TextMessageStart {
                    message_id: message_id.clone(),
                    subagent_run_id: Some(invocation.clone()),
                })
                .await;
        if !started {
            return None;
        }
        let instruction =
            teams::member_instruction(member, &self.turn.members, moderated, "the user");
        let prompt =
            teams::member_prompt(&self.history, &self.turn.prompt, round, member.conductor);
        let mut visible = VisibleStream::default();
        let outcome = self
            .stream_member(
                member,
                &invocation,
                &message_id,
                instruction,
                prompt,
                &mut visible,
            )
            .await?;
        let tail = visible.finish();
        if !tail.is_empty()
            && !self
                .send(AgUiEvent::TextMessageContent {
                    message_id: message_id.clone(),
                    delta: tail,
                })
                .await
        {
            return None;
        }
        let reply = match outcome {
            Ok(()) if teams::is_pass(visible.full_text()) => Reply::Passed,
            Ok(()) => Reply::Spoke(visible.full_text().to_owned()),
            Err(error) => {
                tracing::warn!(member = %member.id, %error, "team member failed to answer");
                Reply::Failed
            }
        };
        let (outcome, state) = match &reply {
            Reply::Spoke(text) => {
                let shown = if member.conductor {
                    teams::parse_conductor_reply(text).0
                } else {
                    text.trim().to_owned()
                };
                self.history.push(TranscriptLine {
                    source: None,
                    speaker: member.name.clone(),
                    text: shown,
                });
                (SubagentOutcome::Success, SpeakerState::Done)
            }
            Reply::Passed => (SubagentOutcome::Passed, SpeakerState::Passed),
            Reply::Failed => (SubagentOutcome::Failed, SpeakerState::Failed),
        };
        self.activity.set(&member.id, state);
        let finished = self.send(AgUiEvent::TextMessageEnd { message_id }).await
            && self
                .send(AgUiEvent::SubagentFinished {
                    subagent_run_id: invocation,
                    outcome,
                })
                .await
            && self.snapshot().await;
        finished.then_some(reply)
    }

    /// Streams one member's model output; the outer `None` means the UI is gone.
    async fn stream_member(
        &self,
        member: &TeamMember,
        invocation: &str,
        message_id: &str,
        instruction: String,
        prompt: String,
        visible: &mut VisibleStream,
    ) -> Option<Result<(), String>> {
        let runner = match self
            .harness
            .member_runner(member, instruction, invocation)
            .await
        {
            Ok(runner) => runner,
            Err(error) => return Some(Err(self.harness.safe_error(&error))),
        };
        let harness = self.harness;
        let result = async {
            let mut stream = runner
                .run_str(
                    DEFAULT_USER_ID,
                    invocation,
                    Content::new("user").with_text(&prompt),
                )
                .await
                .map_err(|error| harness.safe_error(&error))?;
            let mut deltas = EventTextDeltas::default();
            while let Some(item) = stream.next().await {
                let event = item.map_err(|error| harness.safe_error(&error))?;
                let delta_event = deltas.push(&event);
                for widget in crate::artifacts::saved_artifact_widgets(&delta_event) {
                    if self
                        .events
                        .send(Event::AgUiSurface {
                            session_id: self.turn.session_id.clone(),
                            widget,
                        })
                        .await
                        .is_err()
                    {
                        return Ok(false);
                    }
                }
                if let (_, Some(text)) = extract_output(&delta_event) {
                    let shown = visible.push(&text);
                    if !shown.is_empty()
                        && !self
                            .send(AgUiEvent::TextMessageContent {
                                message_id: message_id.to_owned(),
                                delta: shown,
                            })
                            .await
                    {
                        return Ok(false);
                    }
                }
            }
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => Some(Ok(())),
            Ok(false) => None,
            Err(message) => Some(Err(message)),
        }
    }
}

impl AdkHarness {
    /// A plain single agent for one member invocation, on the same model, in its
    /// own throwaway session store: the shared history travels in the prompt, so
    /// nothing outlives the reply. Members get the same scoped knowledge readers
    /// plus their own artifact saver; unrelated parent tools are not inherited.
    async fn member_runner(
        &self,
        member: &TeamMember,
        instruction: String,
        invocation: &str,
    ) -> Result<Runner, crate::Error> {
        let mut builder = LlmAgentBuilder::new(member.id.clone())
            .description(member.role.clone())
            .instruction(instruction)
            .global_instruction(&self.standing_instruction)
            .model(Arc::clone(&self.llm));
        for tool in crate::capabilities::shared_knowledge_tools(&self.base_tools) {
            builder = builder.tool(tool);
        }
        if let Some(saver) = &self.artifact_saver {
            builder = builder.tool(crate::artifacts::save_artifact_tool(
                saver.clone(),
                &member.name,
            ));
        }
        let agent = builder.build().map_err(crate::Error::AgentBuild)?;
        let sessions = Arc::new(InMemorySessionService::new());
        sessions
            .create(CreateRequest {
                app_name: self.app_name.clone(),
                user_id: DEFAULT_USER_ID.to_owned(),
                session_id: Some(invocation.to_owned()),
                state: HashMap::new(),
            })
            .await
            .map_err(crate::Error::Session)?;
        Runner::builder()
            .app_name(&self.app_name)
            .agent(Arc::new(agent))
            .session_service(sessions as Arc<dyn SessionService>)
            .run_config(crate::run_config())
            .build()
            .map_err(crate::Error::RunnerBuild)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream};
    use sovra_core::teams::{SpeakerState, default_team};
    use sovra_core::{AgUiEvent, AgentConfig, Event, SubagentOutcome};

    use super::TeamTurn;
    use crate::AdkHarness;

    /// Answers by persona: the Conductor closes, Kai passes, others speak in two chunks.
    struct TeamLlm;

    #[async_trait::async_trait]
    impl Llm for TeamLlm {
        fn name(&self) -> &'static str {
            "team-test"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let seen = format!("{request:?}");
            let chunks: Vec<&str> = if seen.contains("MODERATOR PROTOCOL") {
                vec![
                    "We agree: soup. ",
                    "[[CONDUCTOR: {\"verdict\":\"YIELD_TO_USER\",\"next_speakers\":[]}]]",
                ]
            } else if seen.contains("You are Kai") {
                vec!["<pa", "ss/>"]
            } else {
                vec!["Try ", "soup."]
            };
            let responses: Vec<adk_core::Result<LlmResponse>> = chunks
                .into_iter()
                .map(|text| {
                    Ok(LlmResponse {
                        content: Some(Content::new("model").with_text(text)),
                        partial: true,
                        ..LlmResponse::default()
                    })
                })
                .collect();
            Ok(Box::pin(futures::stream::iter(responses)))
        }
    }

    fn run(prompt: &str) -> Vec<AgUiEvent> {
        futures::executor::block_on(async {
            let harness =
                AdkHarness::with_llm_and_tools(&AgentConfig::default(), Arc::new(TeamLlm), vec![])
                    .expect("harness");
            let (tx, rx) = async_channel::unbounded();
            let turn = TeamTurn {
                session_id: "chat".into(),
                run_id: "run1".into(),
                prompt: prompt.into(),
                history: vec![sovra_core::teams::TranscriptLine {
                    source: None,
                    speaker: "User".into(),
                    text: prompt.into(),
                }],
                members: default_team(),
            };
            harness.run_team(&turn, &tx).await;
            drop(tx);
            let mut events = Vec::new();
            while let Ok(event) = rx.recv().await {
                match event {
                    Event::AgUi { session_id, event } => {
                        assert_eq!(session_id, "chat");
                        events.push(event);
                    }
                    other => panic!("team runs speak AG-UI only: {other:?}"),
                }
            }
            events
        })
    }

    fn text_of(events: &[AgUiEvent], message: &str) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent { message_id, delta } if message_id == message => {
                    Some(delta.as_str())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn moderated_round_streams_members_hides_passes_and_the_verdict() {
        let events = run("What do Priya and Kai think about dinner?");
        assert!(
            matches!(events.first(), Some(AgUiEvent::RunStarted { run_id }) if run_id == "run1")
        );
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
        let started: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started, ["priya", "kai", "alex"]);
        assert_eq!(text_of(&events, "run1-1-reply"), "Try soup.");
        assert_eq!(text_of(&events, "run1-2-reply"), "", "a pass shows nothing");
        assert_eq!(text_of(&events, "run1-3-reply"), "We agree: soup. ");
        let outcomes: Vec<SubagentOutcome> = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentFinished { outcome, .. } => Some(*outcome),
                _ => None,
            })
            .collect();
        assert_eq!(
            outcomes,
            [
                SubagentOutcome::Success,
                SubagentOutcome::Passed,
                SubagentOutcome::Success
            ]
        );
        let Some(AgUiEvent::ActivitySnapshot { activity, .. }) = events
            .iter()
            .rev()
            .find(|event| matches!(event, AgUiEvent::ActivitySnapshot { .. }))
        else {
            panic!("the team card is kept up to date");
        };
        let sovra_core::LiveActivity::Team(activity) = activity else {
            panic!("team runs emit the team card");
        };
        assert!(activity.finished);
        let states: Vec<SpeakerState> = activity.speakers.iter().map(|row| row.state).collect();
        assert_eq!(
            states,
            [SpeakerState::Done, SpeakerState::Passed, SpeakerState::Done]
        );
    }

    #[test]
    fn a_single_member_answers_without_a_moderator() {
        let events = run("@jamie write a thank-you card");
        let started: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started, ["jamie"]);
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
    }
}
