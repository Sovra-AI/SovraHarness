//! Runs a team turn: members answer in rounds, a Conductor moderates, and every
//! step streams to the UI as AG-UI events (`RUN_STARTED`, `ACTIVITY_SNAPSHOT`,
//! `SUBAGENT_STARTED`, `TEXT_MESSAGE_*`, `SUBAGENT_FINISHED`, `RUN_FINISHED`).
//! The rules — who speaks, passes, verdicts — are pure in `sovra_core::teams`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, EventTextDeltas, Part};
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use async_channel::Sender;
use futures::StreamExt as _;
use sovra_core::teams::{
    self, MAX_REPLIES_PER_TURN, MAX_ROUNDS, SpeakerState, TeamActivity, TeamMember, TeamRoom,
    TranscriptLine, Verdict, VisibleStream,
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
    /// Helpers in this chat. Routing never looks outside it.
    pub room: TeamRoom,
}

fn signatures_of(replies: &[Reply]) -> Vec<BTreeSet<String>> {
    replies
        .iter()
        .filter_map(|reply| match reply {
            Reply::Spoke(text) => {
                let signature = teams::round_signature(text);
                (!signature.is_empty()).then_some(signature)
            }
            Reply::Passed | Reply::Failed => None,
        })
        .collect()
}

/// How one member's reply ended.
enum Reply {
    Spoke(String),
    Passed,
    Failed,
}

/// A member with a reply slot this round: its AG-UI ids are fixed up front.
struct Slot<'m> {
    member: &'m TeamMember,
    invocation: String,
    message_id: String,
}

struct TeamRunState<'a> {
    harness: &'a AdkHarness,
    turn: &'a TeamTurn,
    events: &'a Sender<Event>,
    /// Shared by the members answering side by side; locked only between awaits.
    activity: Mutex<TeamActivity>,
    history: Vec<TranscriptLine>,
    replies: usize,
    activity_id: String,
    /// Switched-on helpers in this chat, including the lead when they moderate.
    chat: Vec<TeamMember>,
}

impl AdkHarness {
    /// Streams a whole team turn. Returns when the run finished or the UI went away.
    ///
    /// Within a round, members answer **side by side**: each sees the conversation
    /// as it stood when the round began, and their replies stream at the same time.
    /// The Conductor then reads every reply and decides whether another round helps.
    pub async fn run_team(&self, turn: &TeamTurn, events: &Sender<Event>) {
        let speakers =
            teams::route_speakers(&turn.prompt, &turn.members, &turn.room, &turn.history);
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
        let chat: Vec<TeamMember> = teams::chat_members(&turn.members, &turn.room)
            .into_iter()
            .cloned()
            .collect();
        let chat_ids: HashSet<String> = chat.iter().map(|member| member.id.clone()).collect();
        let mut state = TeamRunState {
            harness: self,
            turn,
            events,
            activity: Mutex::new(TeamActivity::new(1, &turn.members, &speakers)),
            history,
            replies: 0,
            activity_id: format!("{}-team", turn.run_id),
            chat,
        };
        if let Some(lead) = &conductor {
            state
                .activity_mut()
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
        let mut previous_round: Vec<BTreeSet<String>> = Vec::new();
        for round in 1..=MAX_ROUNDS {
            state.activity_mut().round = round;
            let members: Vec<&TeamMember> = speakers
                .iter()
                .filter(|id| chat_ids.contains(id.as_str()))
                .filter_map(|id| {
                    turn.members
                        .iter()
                        .find(|member| member.id == *id && !member.conductor)
                })
                .collect();
            let Some(replies) = state.speak_together(&members, round, moderated).await else {
                return;
            };
            spoke_any |= replies.iter().any(|reply| matches!(reply, Reply::Spoke(_)));
            let Some(lead) = &conductor else {
                break;
            };
            // The lead already spoke in this list only when they are not the
            // conductor flag; speaking them again here would double the reply.
            let lead_only = [lead];
            let Some(reply) = state
                .speak_together(&lead_only, round, true)
                .await
                .and_then(|mut replies| replies.pop())
            else {
                return;
            };
            let (lead_text, decision) = match reply {
                Reply::Spoke(text) => {
                    spoke_any = true;
                    let parsed = teams::parse_conductor_reply(&text);
                    (Some(text), parsed.1)
                }
                Reply::Passed | Reply::Failed => break,
            };
            let mut current_round = signatures_of(&replies);
            if let Some(text) = lead_text.as_deref() {
                let signature = teams::round_signature(text);
                if !signature.is_empty() {
                    current_round.push(signature);
                }
            }
            match decision.verdict {
                Verdict::Continue if round < MAX_ROUNDS && state.replies < MAX_REPLIES_PER_TURN => {
                    // A repeating or thin continue is a hand-back, so the next
                    // member round does not start. Round 1 is allowed to go on.
                    if teams::continue_hands_back(round, &decision, &previous_round, &current_round)
                    {
                        break;
                    }
                    previous_round = current_round;
                    // Later rounds stay inside this chat. Naming someone else
                    // does not bring them in; the lead still only moderates.
                    speakers = teams::next_round_speakers(&decision, &state.chat, &speakers);
                    let chat = state.chat.clone();
                    let activity = state.activity_mut();
                    activity.queue(&chat, &speakers);
                    activity.queue(&chat, std::slice::from_ref(&lead.id));
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
        state.activity_mut().finish(closing);
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
    fn activity_mut(&mut self) -> &mut TeamActivity {
        self.activity
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Applies `change` and returns the card to send; the lock never spans an await.
    fn update(&self, change: impl FnOnce(&mut TeamActivity)) -> TeamActivity {
        let mut activity = self.activity.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut activity);
        activity.clone()
    }

    async fn send(&self, event: AgUiEvent) -> bool {
        self.events
            .send(Event::AgUi {
                session_id: self.turn.session_id.clone(),
                event,
            })
            .await
            .is_ok()
    }

    async fn send_card(&self, activity: TeamActivity) -> bool {
        self.send(AgUiEvent::ActivitySnapshot {
            message_id: self.activity_id.clone(),
            activity: LiveActivity::Team(activity),
        })
        .await
    }

    async fn snapshot(&self) -> bool {
        let activity = self.update(|_| {});
        self.send_card(activity).await
    }

    /// One round's members, answering at the same time as AG-UI subagents.
    /// Replies come back in `members` order; `None` means the UI is gone.
    async fn speak_together(
        &mut self,
        members: &[&TeamMember],
        round: u8,
        moderated: bool,
    ) -> Option<Vec<Reply>> {
        // Reply slots are handed out up front, in order, so the cap and every
        // AG-UI id are the same however the replies happen to finish.
        let mut slots = Vec::with_capacity(members.len());
        for member in members {
            if self.replies >= MAX_REPLIES_PER_TURN {
                break;
            }
            self.replies += 1;
            let invocation = format!("{}-{}", self.turn.run_id, self.replies);
            slots.push(Slot {
                member,
                message_id: format!("{invocation}-reply"),
                invocation,
            });
        }
        let ids: Vec<String> = slots.iter().map(|slot| slot.member.id.clone()).collect();
        self.activity_mut().start_together(&ids);
        if !self.snapshot().await {
            return None;
        }
        for slot in &slots {
            let member = slot.member;
            let started = self
                .send(AgUiEvent::SubagentStarted {
                    subagent_run_id: slot.invocation.clone(),
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
                        message_id: slot.message_id.clone(),
                        subagent_run_id: Some(slot.invocation.clone()),
                    })
                    .await;
            if !started {
                return None;
            }
        }
        let shared = &*self;
        let answers = futures::future::join_all(
            slots
                .iter()
                .map(|slot| shared.answer(slot, &shared.history, round, moderated)),
        )
        .await;
        let mut replies = Vec::with_capacity(members.len());
        for (slot, answer) in slots.iter().zip(answers) {
            let reply = answer?;
            if let Reply::Spoke(text) = &reply {
                let shown = if slot.member.conductor {
                    teams::parse_conductor_reply(text).0
                } else {
                    text.trim().to_owned()
                };
                self.history.push(TranscriptLine {
                    source: None,
                    speaker: slot.member.name.clone(),
                    text: shown,
                });
            }
            replies.push(reply);
        }
        // Members past the reply cap say nothing this turn.
        replies.resize_with(members.len(), || Reply::Passed);
        Some(replies)
    }

    /// Streams one member's reply and closes its card as soon as it is done, while
    /// the others may still be answering. `None` means the UI is gone.
    async fn answer(
        &self,
        slot: &Slot<'_>,
        history: &[TranscriptLine],
        round: u8,
        moderated: bool,
    ) -> Option<Reply> {
        let member = slot.member;
        let instruction = teams::member_instruction(member, &self.chat, moderated, "the user");
        let prompt = teams::member_prompt(history, &self.turn.prompt, round, member.conductor);
        let mut visible = VisibleStream::default();
        let outcome = self
            .stream_member(slot, instruction, prompt, &mut visible)
            .await?;
        let tail = visible.finish();
        if !tail.is_empty()
            && !self
                .send(AgUiEvent::TextMessageContent {
                    message_id: slot.message_id.clone(),
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
            Reply::Spoke(_) => (SubagentOutcome::Success, SpeakerState::Done),
            Reply::Passed => (SubagentOutcome::Passed, SpeakerState::Passed),
            Reply::Failed => (SubagentOutcome::Failed, SpeakerState::Failed),
        };
        let card = self.update(|activity| activity.set(&member.id, state));
        let finished = self
            .send(AgUiEvent::TextMessageEnd {
                message_id: slot.message_id.clone(),
            })
            .await
            && self
                .send(AgUiEvent::SubagentFinished {
                    subagent_run_id: slot.invocation.clone(),
                    outcome,
                })
                .await
            && self.send_card(card).await;
        finished.then_some(reply)
    }

    /// Streams one member's model output; the outer `None` means the UI is gone.
    /// Each tool call the member makes shows on its kanban card as it happens.
    async fn stream_member(
        &self,
        slot: &Slot<'_>,
        instruction: String,
        prompt: String,
        visible: &mut VisibleStream,
    ) -> Option<Result<(), String>> {
        let member = slot.member;
        let runner = match self
            .harness
            .member_runner(member, instruction, &slot.invocation)
            .await
        {
            Ok(runner) => runner,
            Err(error) => return Some(Err(self.harness.safe_error(&error))),
        };
        let harness = self.harness;
        let mut seen_calls = HashSet::new();
        let result = async {
            let mut stream = runner
                .run_str(
                    DEFAULT_USER_ID,
                    &slot.invocation,
                    Content::new("user").with_text(&prompt),
                )
                .await
                .map_err(|error| harness.safe_error(&error))?;
            let mut deltas = EventTextDeltas::default();
            while let Some(item) = stream.next().await {
                let event = item.map_err(|error| harness.safe_error(&error))?;
                for label in new_step_labels(&event, &mut seen_calls) {
                    let card = self.update(|activity| activity.set_doing(&member.id, label));
                    if !self.send_card(card).await {
                        return Ok(false);
                    }
                }
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
                                message_id: slot.message_id.clone(),
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

/// Plain-words labels for tool calls in `event` not reported before.
fn new_step_labels(event: &adk_core::Event, seen: &mut HashSet<String>) -> Vec<String> {
    let Some(content) = &event.llm_response.content else {
        return Vec::new();
    };
    let mut labels = Vec::new();
    for (index, part) in content.parts.iter().enumerate() {
        if let Part::FunctionCall { name, args, id, .. } = part {
            if id.is_none() && event.llm_response.partial {
                continue;
            }
            let key = id.clone().unwrap_or_else(|| format!("{name}#{index}"));
            if seen.insert(key) {
                labels.push(sovra_core::work::describe_step(name, args).1);
            }
        }
    }
    labels
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
        let mut tools: Vec<Arc<dyn adk_core::Tool>> =
            crate::capabilities::shared_knowledge_tools(&self.base_tools).collect();
        if let Some(saver) = &self.artifact_saver {
            tools.push(crate::artifacts::save_artifact_tool(
                saver.clone(),
                &member.name,
            ));
        }
        // Safe reads run together; anything with side effects stays in order.
        let strategy = crate::orchestrator::execution_strategy(&tools);
        let mut builder = crate::work_log::harden(
            LlmAgentBuilder::new(member.id.clone())
                .description(member.role.clone())
                .instruction(instruction)
                .global_instruction(&self.standing_instruction)
                .model(Arc::clone(&self.llm))
                .tool_execution_strategy(strategy),
            sovra_core::work::MEMBER_MAX_STEPS,
        );
        for tool in tools {
            builder = builder.tool(tool);
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
    use sovra_core::teams::{SpeakerState, TeamRoom, default_team};
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
        run_with(
            prompt,
            default_team(),
            TeamRoom::all_helpers(),
            Arc::new(TeamLlm),
        )
    }

    fn run_with(
        prompt: &str,
        members: Vec<sovra_core::teams::TeamMember>,
        room: TeamRoom,
        llm: Arc<dyn Llm>,
    ) -> Vec<AgUiEvent> {
        futures::executor::block_on(async {
            let harness = AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, vec![])
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
                members,
                room,
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
    fn a_rounds_members_start_together_and_show_on_the_board() {
        let events = run("What do Priya and Kai think about dinner?");
        let first_finish = events
            .iter()
            .position(|event| matches!(event, AgUiEvent::SubagentFinished { .. }))
            .expect("members finish");
        let started_before: Vec<&str> = events[..first_finish]
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(started_before, ["priya", "kai"], "both answer side by side");
        let working_together = events.iter().any(|event| {
            matches!(event, AgUiEvent::ActivitySnapshot {
                activity: sovra_core::LiveActivity::Team(card), ..
            } if card.lanes().working.len() == 2 && card.status.contains("at the same time"))
        });
        assert!(
            working_together,
            "the board shows both members working at once"
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

    #[test]
    fn a_complex_question_asks_many_helpers_and_one_helper_twice() {
        let example = sovra_core::skills::builtin_skills()
            .into_iter()
            .find(|skill| skill.name == "team")
            .expect("team skill")
            .friendly_card_copy()
            .sample_draft;
        let prompt = sovra_core::teams::strip_team_command(&example)
            .expect("the skills example asks the team");
        let events = run_with(
            prompt,
            default_team(),
            TeamRoom::ordinary(),
            Arc::new(TeamLlm),
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgUiEvent::RunError { .. })),
            "{events:?}"
        );
        let first_finish = events
            .iter()
            .position(|event| matches!(event, AgUiEvent::SubagentFinished { .. }))
            .expect("helpers finish");
        let side_by_side = started_names(&events[..first_finish]);
        assert_eq!(
            side_by_side,
            ["priya", "priya", "sam", "jamie", "morgan", "robin", "kai"],
            "the jobs answer together, with Priya on two of them"
        );
        let priya_runs: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentStarted {
                    name,
                    subagent_run_id,
                    ..
                } if name == "priya" => Some(subagent_run_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(priya_runs.len(), 2);
        assert_ne!(priya_runs[0], priya_runs[1]);
        let two_priyas = events.iter().any(|event| {
            matches!(event, AgUiEvent::ActivitySnapshot {
                activity: sovra_core::LiveActivity::Team(card), ..
            } if card.speakers.iter().filter(|row| {
                row.member_id == "priya" && row.state == SpeakerState::Speaking
            }).count() == 2)
        });
        assert!(two_priyas, "both of Priya's jobs show as answering");
        let named_pass = events.iter().any(|event| {
            matches!(event, AgUiEvent::ActivitySnapshot {
                activity: sovra_core::LiveActivity::Team(card), ..
            } if card.status.contains("Kai: Nothing to add"))
        });
        assert!(named_pass, "a pass is shown by name as nothing to add");
        assert_eq!(
            started_names(&events),
            [
                "priya", "priya", "sam", "jamie", "morgan", "robin", "kai", "alex"
            ]
        );
        let visible: String = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert!(!visible.contains("<pass"), "{visible}");
        assert!(!visible.contains("[[CONDUCTOR:"), "{visible}");
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
    }

    #[test]
    fn an_ordinary_group_question_asks_one_helper_and_not_the_lead() {
        let events = run("hello");
        let started = started_names(&events);
        assert_eq!(started, ["morgan"]);
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
    }

    /// Same sentence every time, and a continue that names Morgan again.
    struct RepeatingLlm;

    #[async_trait::async_trait]
    impl Llm for RepeatingLlm {
        fn name(&self) -> &'static str {
            "team-repeat"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let seen = format!("{request:?}");
            let text = if seen.contains("MODERATOR PROTOCOL") {
                "Same summary for the room. [[CONDUCTOR: {\"verdict\":\"CONTINUE\",\"next_speakers\":[\"morgan\"],\"confidence\":0.9}]]"
            } else {
                "The garden needs water every morning."
            };
            let responses = vec![Ok(LlmResponse {
                content: Some(Content::new("model").with_text(text)),
                partial: true,
                ..LlmResponse::default()
            })];
            Ok(Box::pin(futures::stream::iter(responses)))
        }
    }

    fn started_names(events: &[AgUiEvent]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::SubagentStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_repeating_continue_finishes_the_round_and_does_not_start_another() {
        let keep = ["morgan", "priya", "sam", "alex"];
        let members = default_team()
            .into_iter()
            .filter(|member| keep.contains(&member.id.as_str()))
            .collect();
        let events = run_with(
            "What does everyone think?",
            members,
            TeamRoom::all_helpers(),
            Arc::new(RepeatingLlm),
        );
        let started = started_names(&events);
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for name in started {
            *counts.entry(name).or_default() += 1;
        }
        assert_eq!(counts.get("morgan").copied(), Some(2));
        assert!(counts.contains_key("alex"));
        assert!(
            counts.values().all(|count| *count < 3),
            "a third member round must not start: {counts:?}"
        );
        let visible: String = events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert!(!visible.contains("<pass"), "{visible}");
        assert!(!visible.contains("[[CONDUCTOR:"), "{visible}");
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
    }

    /// Continues by naming Sam, who is on the roster but not in this chat.
    struct OutsiderLlm {
        seen: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Llm for OutsiderLlm {
        fn name(&self) -> &'static str {
            "team-outsider"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let seen = format!("{request:?}");
            self.seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(seen.clone());
            let text = if seen.contains("MODERATOR PROTOCOL") {
                "Here is a fresh summary for the room. [[CONDUCTOR: {\"verdict\":\"CONTINUE\",\"next_speakers\":[\"sam\"],\"confidence\":0.9}]]"
            } else {
                "The garden needs water every morning."
            };
            let responses = vec![Ok(LlmResponse {
                content: Some(Content::new("model").with_text(text)),
                partial: true,
                ..LlmResponse::default()
            })];
            Ok(Box::pin(futures::stream::iter(responses)))
        }
    }

    #[test]
    fn a_chosen_group_continue_does_not_call_someone_outside_it() {
        let llm = Arc::new(OutsiderLlm {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let events = run_with(
            "What does everyone think?",
            default_team(),
            TeamRoom::group(["morgan", "priya"]),
            llm.clone(),
        );
        let started = started_names(&events);
        assert!(
            !started.contains(&"sam"),
            "Sam is not in this chat: {started:?}"
        );
        assert_eq!(
            started.iter().filter(|name| **name == "morgan").count(),
            2,
            "the continue must start another round inside the chat: {started:?}"
        );
        assert_eq!(
            started.iter().filter(|name| **name == "priya").count(),
            2,
            "{started:?}"
        );
        assert!(started.contains(&"alex"), "{started:?}");
        assert!(
            started
                .iter()
                .all(|name| matches!(*name, "morgan" | "priya" | "alex")),
            "{started:?}"
        );
        let seen = llm
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let moderator: Vec<&String> = seen
            .iter()
            .filter(|text| text.contains("MODERATOR PROTOCOL"))
            .collect();
        assert!(!moderator.is_empty(), "the lead moderated the turn");
        for text in moderator {
            assert!(text.contains("@morgan"), "{text}");
            assert!(text.contains("@priya"), "{text}");
            assert!(!text.contains("@sam"), "{text}");
        }
        assert!(matches!(events.last(), Some(AgUiEvent::RunFinished { .. })));
    }
}
