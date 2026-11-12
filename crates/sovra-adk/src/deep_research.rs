//! Parallel deep-research team run.
//!
//! Always runs as an in-chat AG-UI board (waiting / working / done), researches
//! independent goals concurrently, then saves one Document artifact so the
//! transcript stays clean.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, EventTextDeltas, Tool};
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use async_channel::Sender;
use futures::StreamExt as _;
use futures::future::join_all;
use sovra_core::artifacts::{ArtifactDraft, ArtifactKind, ArtifactSource};
use sovra_core::{
    AgUiEvent, AgUiWidget, Event, LiveActivity, ParsedResearchGoal, ResearchBoard,
    ResearchGoalKind, ResearchGoalState, parse_research_plan, research_board_title,
};

use crate::{AdkHarness, DEFAULT_USER_ID, extract_output};

/// One deep-research turn handed over by the runtime.
#[derive(Debug, Clone)]
pub struct DeepResearchTurn {
    pub session_id: String,
    pub run_id: String,
    pub prompt: String,
}

struct ResearchRun<'a> {
    harness: &'a AdkHarness,
    turn: &'a DeepResearchTurn,
    events: &'a Sender<Event>,
    board: Mutex<ResearchBoard>,
    activity_id: String,
}

impl AdkHarness {
    /// Plan → parallel research goals → compose & save a Document. Streams only
    /// board snapshots plus a short closing note and the artifact card.
    pub async fn run_deep_research(&self, turn: &DeepResearchTurn, events: &Sender<Event>) {
        let run = ResearchRun {
            harness: self,
            turn,
            events,
            board: Mutex::new(ResearchBoard::planning(research_board_title(&turn.prompt))),
            activity_id: format!("{}-research", turn.run_id),
        };
        if !run
            .send(AgUiEvent::RunStarted {
                run_id: turn.run_id.clone(),
            })
            .await
            || !run.snapshot().await
        {
            return;
        }

        let plan_text = match run
            .collect_text(
                "plan",
                PLAN_INSTRUCTION,
                &format!(
                    "Create a research plan for this question. Output only the bullet list.\n\n{}",
                    turn.prompt
                ),
                false,
            )
            .await
        {
            Some(Ok(text)) => text,
            Some(Err(message)) => {
                run.fail(&message).await;
                return;
            }
            None => return,
        };

        let parsed = parse_research_plan(&plan_text);
        let goals: Vec<_> = if parsed.is_empty() {
            vec![
                ParsedResearchGoal {
                    index: 0,
                    kind: ResearchGoalKind::Research,
                    text: turn.prompt.trim().to_owned(),
                }
                .into_goal(),
            ]
        } else {
            parsed
                .into_iter()
                .map(ParsedResearchGoal::into_goal)
                .collect()
        };

        {
            let mut board = run
                .board
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            board.with_plan(goals.clone());
        }
        if !run.snapshot().await {
            return;
        }

        let research_goals: Vec<_> = goals
            .iter()
            .filter(|g| g.kind == ResearchGoalKind::Research)
            .cloned()
            .collect();
        let deliverable_goals: Vec<_> = goals
            .iter()
            .filter(|g| g.kind == ResearchGoalKind::Deliverable)
            .cloned()
            .collect();

        let findings = run.research_parallel(&research_goals).await;
        let Some(mut findings) = findings else {
            return;
        };

        for goal in &deliverable_goals {
            run.set_goal(&goal.id, ResearchGoalState::Working);
            if !run.snapshot().await {
                return;
            }
            let research_blob = findings.join("\n\n---\n\n");
            let prompt = format!(
                "Using the research findings below, complete this deliverable:\n{}\n\nFindings:\n{research_blob}",
                goal.text
            );
            match run
                .collect_text(&goal.id, DELIVERABLE_INSTRUCTION, &prompt, false)
                .await
            {
                Some(Ok(text)) => {
                    findings.push(text);
                    run.set_goal(&goal.id, ResearchGoalState::Done);
                }
                Some(Err(_)) => {
                    run.set_goal(&goal.id, ResearchGoalState::Failed);
                }
                None => return,
            }
            if !run.snapshot().await {
                return;
            }
        }

        {
            let mut board = run
                .board
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            board.begin_writing();
        }
        if !run.snapshot().await {
            return;
        }

        let findings_blob = findings.join("\n\n---\n\n");
        let compose_prompt = format!(
            "Write a clear, cited research document answering:\n{}\n\nUse only these findings. Prefer short sections with headings and a Sources section listing titles and URLs that appear in the findings. Do not invent sources.\n\nFindings:\n{findings_blob}",
            turn.prompt
        );
        let document = match run
            .collect_text("compose", COMPOSE_INSTRUCTION, &compose_prompt, false)
            .await
        {
            Some(Ok(text)) if !text.trim().is_empty() => text,
            Some(Ok(_) | Err(_)) => format!(
                "# Research notes\n\nQuestion: {}\n\n{findings_blob}",
                turn.prompt
            ),
            None => return,
        };

        let title = document_title(&turn.prompt);
        let artifact_id = match run.save_document(&title, &document).await {
            Some(Ok(id)) => id,
            Some(Err(message)) => {
                // Still show the document inline if saving failed.
                if !run
                    .send_closing(
                        &format!("I finished the research but could not save the document: {message}\n\n{document}"),
                        None,
                    )
                    .await
                {
                    return;
                }
                run.finish_ok().await;
                return;
            }
            None => return,
        };

        if !run
            .send_closing(
                "Your research document is ready. Open it below — intermediate notes stay on the board so the chat stays tidy.",
                Some((artifact_id, title)),
            )
            .await
        {
            return;
        }
        run.finish_ok().await;
    }
}

const PLAN_INSTRUCTION: &str = "\
You are a research planner. Create a research plan, not a summary.

RULES:
- Output a bulleted list of 3 to 5 action-oriented research goals.
- Each goal starts with a verb: Investigate, Compare, Analyse, Check, etc.
- Prefix each with [RESEARCH] or [DELIVERABLE].
- [RESEARCH]: information gathering.
- [DELIVERABLE]: synthesis written after research (at most one).
- Do not research the content yourself — only list the goals.
";

const RESEARCH_INSTRUCTION: &str = "\
You are one research helper on a team. Answer only the assigned research goal.
Use web_search when you need current facts. Cite only titles and URLs the tools return.
Write concise findings with clear bullets and a short Sources list. Do not invent sources.
";

const DELIVERABLE_INSTRUCTION: &str = "\
You synthesise research findings into the requested deliverable.
Use only the findings provided. Keep citations that already appear there.
";

const COMPOSE_INSTRUCTION: &str = "\
You write the final research document for a non-technical reader.
Use Markdown with clear headings. Include a Sources section.
Use only the findings provided; never invent titles or URLs.
";

impl ResearchRun<'_> {
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
        let board = self
            .board
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        self.send(AgUiEvent::ActivitySnapshot {
            message_id: self.activity_id.clone(),
            activity: LiveActivity::Research(board),
        })
        .await
    }

    fn set_goal(&self, goal_id: &str, state: ResearchGoalState) {
        self.board
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(goal_id, state);
    }

    async fn fail(&self, message: &str) {
        {
            let mut board = self
                .board
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            board.finish("Research could not finish");
        }
        let _ = self.snapshot().await;
        let _ = self
            .send(AgUiEvent::RunError {
                message: message.into(),
            })
            .await;
    }

    async fn finish_ok(&self) {
        {
            let mut board = self
                .board
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            board.finish("Your research document is ready");
        }
        if self.snapshot().await {
            let _ = self
                .send(AgUiEvent::RunFinished {
                    run_id: self.turn.run_id.clone(),
                })
                .await;
        }
    }

    /// Runs research goals concurrently; updates the board as each finishes.
    /// Returns `None` if the UI disconnected.
    async fn research_parallel(&self, goals: &[sovra_core::ResearchGoal]) -> Option<Vec<String>> {
        if goals.is_empty() {
            return Some(Vec::new());
        }
        for goal in goals {
            self.set_goal(&goal.id, ResearchGoalState::Working);
        }
        if !self.snapshot().await {
            return None;
        }

        let tasks = goals.iter().map(|goal| {
            let goal = goal.clone();
            async move {
                let prompt = format!(
                    "Research goal:\n{}\n\nOverall question:\n{}",
                    goal.text, self.turn.prompt
                );
                let result = self
                    .collect_text(&goal.id, RESEARCH_INSTRUCTION, &prompt, true)
                    .await;
                (goal.id, result)
            }
        });

        let results = join_all(tasks).await;
        let mut findings = Vec::new();
        let mut alive = true;
        for (goal_id, result) in results {
            match result {
                Some(Ok(text)) => {
                    findings.push(format!("## Goal findings\n\n{text}"));
                    self.set_goal(&goal_id, ResearchGoalState::Done);
                    self.board
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .set_note(&goal_id, "Findings ready");
                }
                Some(Err(_)) => {
                    self.set_goal(&goal_id, ResearchGoalState::Failed);
                }
                None => {
                    alive = false;
                }
            }
            if alive && !self.snapshot().await {
                alive = false;
            }
        }
        alive.then_some(findings)
    }

    /// Runs one isolated helper and returns its full answer text (not streamed to chat).
    async fn collect_text(
        &self,
        invocation_suffix: &str,
        instruction: &str,
        prompt: &str,
        with_search: bool,
    ) -> Option<Result<String, String>> {
        let invocation = format!("{}-{invocation_suffix}", self.turn.run_id);
        let runner = match self
            .harness
            .research_runner(instruction, &invocation, with_search)
            .await
        {
            Ok(runner) => runner,
            Err(error) => return Some(Err(self.harness.safe_error(&error))),
        };
        let harness = self.harness;
        let collected = async {
            let mut stream = runner
                .run_str(
                    DEFAULT_USER_ID,
                    &invocation,
                    Content::new("user").with_text(prompt),
                )
                .await
                .map_err(|error| harness.safe_error(&error))?;
            let mut deltas = EventTextDeltas::default();
            let mut text = String::new();
            while let Some(item) = stream.next().await {
                let event = item.map_err(|error| harness.safe_error(&error))?;
                let delta_event = deltas.push(&event);
                if let (_, Some(chunk)) = extract_output(&delta_event) {
                    text.push_str(&chunk);
                }
            }
            Ok(text)
        }
        .await;
        Some(collected)
    }

    async fn save_document(&self, title: &str, content: &str) -> Option<Result<String, String>> {
        let Some(saver) = &self.harness.artifact_saver else {
            return Some(Err(
                "Saving documents is unavailable in this session.".into()
            ));
        };
        let draft = ArtifactDraft {
            title: title.to_owned(),
            kind: ArtifactKind::Document,
            language: String::new(),
            content: content.to_owned(),
            source: ArtifactSource {
                session_id: self.turn.session_id.clone(),
                message_id: self.activity_id.clone(),
                author: "Research".into(),
                shared_from: String::new(),
            },
        };
        match saver.save(draft).await {
            Ok(artifact) => Some(Ok(artifact.id)),
            Err(message) => Some(Err(message)),
        }
    }

    async fn send_closing(&self, text: &str, artifact: Option<(String, String)>) -> bool {
        let message_id = format!("{}-closing", self.turn.run_id);
        if !self
            .send(AgUiEvent::TextMessageStart {
                message_id: message_id.clone(),
                subagent_run_id: None,
            })
            .await
        {
            return false;
        }
        if !self
            .send(AgUiEvent::TextMessageContent {
                message_id: message_id.clone(),
                delta: text.into(),
            })
            .await
        {
            return false;
        }
        if !self
            .send(AgUiEvent::TextMessageEnd {
                message_id: message_id.clone(),
            })
            .await
        {
            return false;
        }
        if let Some((artifact_id, title)) = artifact
            && self
                .events
                .send(Event::AgUiSurface {
                    session_id: self.turn.session_id.clone(),
                    widget: AgUiWidget::ArtifactSaved {
                        artifact_id,
                        title,
                        artifact_kind: ArtifactKind::Document,
                    },
                })
                .await
                .is_err()
        {
            return false;
        }
        true
    }
}

impl AdkHarness {
    async fn research_runner(
        &self,
        instruction: &str,
        invocation: &str,
        with_search: bool,
    ) -> Result<Runner, crate::Error> {
        let mut builder = crate::work_log::harden(
            LlmAgentBuilder::new("DeepResearchHelper")
                .description("Researches one goal on the deep-research board.")
                .instruction(instruction)
                .global_instruction(&self.standing_instruction)
                .model(Arc::clone(&self.llm)),
            sovra_core::work::HELPER_MAX_STEPS,
        );
        if with_search {
            for tool in search_tools(&self.base_tools) {
                builder = builder.tool(tool);
            }
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

fn search_tools(tools: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
    tools
        .iter()
        .filter(|tool| {
            let name = tool.name();
            name == crate::web_search::WEB_SEARCH_TOOL_NAME
                || name == crate::web_search::READ_WEB_PAGE_TOOL_NAME
        })
        .cloned()
        .collect()
}

fn document_title(prompt: &str) -> String {
    let words: Vec<&str> = prompt.split_whitespace().take(8).collect();
    if words.is_empty() {
        "Research report".into()
    } else {
        let mut title = words.join(" ");
        if prompt.split_whitespace().count() > 8 {
            title.push('…');
        }
        title
    }
}

/// Unique-ish run id for tests and one-shot launches.
pub fn new_research_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("research-{millis}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use adk_core::{Content, Llm, LlmRequest, LlmResponse, LlmResponseStream};
    use sovra_core::artifacts::Artifact;
    use sovra_core::{
        AgUiEvent, AgUiWidget, AgentConfig, Event, LiveActivity, ResearchBoard, ResearchGoalState,
        Timestamp,
    };

    use super::{DeepResearchTurn, new_research_run_id};
    use crate::AdkHarness;
    use crate::artifacts::ArtifactSaver;
    use crate::web_search::SearchClient;

    /// Scripted model: plan → per-goal findings → compose. Records overlap.
    struct ResearchLlm {
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        goal_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Llm for ResearchLlm {
        fn name(&self) -> &'static str {
            "deep-research-test"
        }

        async fn generate_content(
            &self,
            request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let seen = format!("{request:?}");
            let text = if seen.contains("Create a research plan") {
                "- [RESEARCH] Investigate heat pump efficiency ratings\n\
                 - [RESEARCH] Compare running costs versus gas boilers\n\
                 - [DELIVERABLE] Write a homeowner summary table\n"
                    .to_owned()
            } else if seen.contains("Write a clear, cited research document")
                || seen.contains("final research document")
            {
                "# Heat pumps vs gas boilers\n\nHeat pumps win on running cost in most climates.\n\n## Sources\n\n- Example Source — https://example.test/heat-pumps\n".to_owned()
            } else if seen.contains("complete this deliverable") {
                "| Option | Running cost |\n| --- | --- |\n| Heat pump | Lower |\n| Gas boiler | Higher |\n".to_owned()
            } else {
                let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(current, Ordering::SeqCst);
                self.goal_calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                self.active.fetch_sub(1, Ordering::SeqCst);
                "Findings with a source.\n\nSources:\n- Example Source — https://example.test/heat-pumps\n"
                    .to_owned()
            };
            let responses = vec![Ok(LlmResponse {
                content: Some(Content::new("model").with_text(text)),
                partial: true,
                ..LlmResponse::default()
            })];
            Ok(Box::pin(futures::stream::iter(responses)))
        }
    }

    async fn run_research() -> (Vec<Event>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let goal_calls = Arc::new(AtomicUsize::new(0));
        let llm = Arc::new(ResearchLlm {
            active: Arc::clone(&active),
            max_active: Arc::clone(&max_active),
            goal_calls: Arc::clone(&goal_calls),
        });
        let base =
            AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, vec![]).expect("harness");
        let search = SearchClient::from_fn(|_query| async {
            Ok(r#"{"results":[{"title":"Example Source","url":"https://example.test/heat-pumps","snippet":"Efficient.","sources":["test"]}]}"#.into())
        });
        let harness = base.with_web_search(search).expect("search");
        let saved = Arc::new(AtomicUsize::new(0));
        let saved_count = Arc::clone(&saved);
        let harness = harness
            .with_artifact_saver(ArtifactSaver::from_fn(move |draft| {
                let saved_count = Arc::clone(&saved_count);
                async move {
                    saved_count.fetch_add(1, Ordering::SeqCst);
                    Ok(Artifact::from_draft("doc-1".into(), draft, Timestamp(1)))
                }
            }))
            .expect("saver");
        let (tx, rx) = async_channel::unbounded();
        let turn = DeepResearchTurn {
            session_id: "chat".into(),
            run_id: new_research_run_id(),
            prompt: "Compare heat pumps vs gas boilers for a small family home".into(),
        };
        harness.run_deep_research(&turn, &tx).await;
        drop(tx);
        let mut out = Vec::new();
        while let Ok(event) = rx.recv().await {
            out.push(event);
        }
        assert_eq!(saved.load(Ordering::SeqCst), 1, "document must be saved");
        (out, max_active, goal_calls)
    }

    #[tokio::test]
    async fn parallel_goals_overlap_emit_board_and_save_document() {
        let (events, max_active, goal_calls) = run_research().await;
        assert!(
            max_active.load(Ordering::SeqCst) >= 2,
            "independent research goals must overlap in time"
        );
        assert!(
            goal_calls.load(Ordering::SeqCst) >= 2,
            "each research goal must run"
        );

        let agui: Vec<&AgUiEvent> = events
            .iter()
            .filter_map(|event| match event {
                Event::AgUi { event, .. } => Some(event),
                _ => None,
            })
            .collect();
        assert!(
            matches!(agui.first(), Some(AgUiEvent::RunStarted { .. })),
            "{agui:?}"
        );
        assert!(
            matches!(agui.last(), Some(AgUiEvent::RunFinished { .. })),
            "{agui:?}"
        );

        let boards: Vec<ResearchBoard> = agui
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::ActivitySnapshot {
                    activity: LiveActivity::Research(board),
                    ..
                } => Some(board.clone()),
                _ => None,
            })
            .collect();
        assert!(
            boards
                .iter()
                .any(|board| board.working_count() >= 2 || board.lanes().working.len() >= 2),
            "board must expose concurrent working lanes: {boards:?}"
        );
        assert!(
            boards.last().is_some_and(|board| {
                board.finished
                    && board
                        .goals
                        .iter()
                        .filter(|goal| {
                            matches!(
                                goal.state,
                                ResearchGoalState::Done | ResearchGoalState::Failed
                            )
                        })
                        .count()
                        >= 2
            }),
            "final board must finish with done goals: {boards:?}"
        );

        assert!(
            events.iter().any(|event| matches!(
                event,
                Event::AgUiSurface {
                    widget: AgUiWidget::ArtifactSaved {
                        artifact_kind: sovra_core::artifacts::ArtifactKind::Document,
                        ..
                    },
                    ..
                }
            )),
            "run must emit a Document artifact card: {events:?}"
        );

        let text_messages: Vec<&str> = agui
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent { delta, .. } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text_messages.len(),
            1,
            "only the closing note should appear in chat text: {text_messages:?}"
        );
        assert!(text_messages[0].contains("research document is ready"));
    }
}
