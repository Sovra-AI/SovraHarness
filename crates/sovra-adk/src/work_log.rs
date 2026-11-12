//! The agent loop's guard rails and its live work log.
//!
//! - [`harden`] gives every agent the same limits (`sovra_core::work`): a step
//!   cap, a circuit breaker for a tool that keeps failing, and a refusal when the
//!   model repeats an identical call.
//! - [`StepTap`] lets a specialist helper's own tool calls reach the screen. ADK's
//!   `AgentTool` forwards a child's events through the parent tool context; the
//!   tap keeps only who called which tool and re-emits that as a metadata-only,
//!   partial event. Partial events are never persisted, so the parent's history
//!   (what the model sees next turn) is unchanged.
//! - [`WorkTracker`] folds the run's events into a `WorkActivity` and streams it
//!   as AG-UI `ACTIVITY_SNAPSHOT` (`activityType: "sovra.work"`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use adk_agent::LlmAgentBuilder;
use adk_core::{
    Artifacts, CallbackContext, Content, EventActions, Memory, MemoryEntry, Part, ReadonlyContext,
    RunConfig, Session, State, ToolContext,
};
use async_channel::Sender;
use serde::{Deserialize, Serialize};
use sovra_core::work::{
    CIRCUIT_BREAKER_FAILURES, REPEATED_CALL_MESSAGE, RepeatGuard, StepCall, WorkActivity,
    helper_display_name,
};
use sovra_core::{AgUiEvent, Event, LiveActivity};

/// `provider_metadata` key of a tapped helper step.
const STEP_KEY: &str = "sovra_step";

/// The same limits for every agent loop; `max_steps` is the model-call cap.
pub(crate) fn harden(builder: LlmAgentBuilder, max_steps: u32) -> LlmAgentBuilder {
    let guard = Arc::new(Mutex::new(RepeatGuard::default()));
    builder
        .max_iterations(max_steps)
        .circuit_breaker_threshold(CIRCUIT_BREAKER_FAILURES)
        .before_tool_callback(Box::new(move |ctx: Arc<dyn CallbackContext>| {
            let guard = Arc::clone(&guard);
            Box::pin(async move {
                let (Some(tool), Some(args)) = (ctx.tool_name(), ctx.tool_input()) else {
                    return Ok(None);
                };
                // A poisoned guard only loses loop detection; never block work on it.
                let allowed = guard.lock().map_or(true, |mut guard| {
                    guard.allow(ctx.invocation_id(), tool, args)
                });
                if allowed {
                    Ok(None)
                } else {
                    tracing::warn!(tool, "refused a repeated identical tool call");
                    Err(adk_core::AdkError::tool(REPEATED_CALL_MESSAGE))
                }
            })
        }))
}

/// One helper step, as the tap reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StepNote {
    /// The helper lane: the id of the delegation call that asked this helper.
    lane: String,
    /// The helper (agent) that made the call.
    helper: String,
    id: String,
    tool: String,
    #[serde(default)]
    args: serde_json::Value,
    /// `None` while starting; `Some(ok)` once the result is back.
    #[serde(default)]
    done: Option<bool>,
}

/// A tool context that passes everything through to the parent, except that a
/// delegated helper's forwarded events become step notes.
pub(crate) struct StepTap {
    inner: Arc<dyn ToolContext>,
}

impl StepTap {
    pub(crate) fn wrap(inner: Arc<dyn ToolContext>) -> Arc<dyn ToolContext> {
        Arc::new(Self { inner })
    }

    fn notes(&self, event: &adk_core::Event) -> Vec<StepNote> {
        let Some(content) = &event.llm_response.content else {
            return Vec::new();
        };
        let scope = self.inner.function_call_id();
        let mut notes = Vec::new();
        for (index, part) in content.parts.iter().enumerate() {
            match part {
                Part::FunctionCall { name, args, id, .. } => notes.push(StepNote {
                    lane: scope.to_owned(),
                    helper: event.author.clone(),
                    id: scoped_id(scope, id.as_deref(), name, index),
                    tool: name.clone(),
                    args: args.clone(),
                    done: None,
                }),
                Part::FunctionResponse {
                    function_response,
                    id,
                    ..
                } => notes.push(StepNote {
                    lane: scope.to_owned(),
                    helper: event.author.clone(),
                    id: scoped_id(scope, id.as_deref(), &function_response.name, index),
                    tool: function_response.name.clone(),
                    args: serde_json::Value::Null,
                    done: Some(!is_error(&function_response.response)),
                }),
                _ => {}
            }
        }
        notes
    }
}

/// Child call ids are unique only inside the child, and helpers run side by side.
fn scoped_id(scope: &str, id: Option<&str>, name: &str, index: usize) -> String {
    match id {
        Some(id) => format!("{scope}/{id}"),
        None => format!("{scope}/{name}#{index}"),
    }
}

fn is_error(response: &serde_json::Value) -> bool {
    response.get("error").is_some_and(|error| !error.is_null())
}

#[async_trait::async_trait]
impl ReadonlyContext for StepTap {
    fn invocation_id(&self) -> &str {
        self.inner.invocation_id()
    }
    fn agent_name(&self) -> &str {
        self.inner.agent_name()
    }
    fn user_id(&self) -> &str {
        self.inner.user_id()
    }
    fn app_name(&self) -> &str {
        self.inner.app_name()
    }
    fn session_id(&self) -> &str {
        self.inner.session_id()
    }
    fn branch(&self) -> &str {
        self.inner.branch()
    }
    fn user_content(&self) -> &Content {
        self.inner.user_content()
    }
    fn state(&self) -> Option<&dyn State> {
        self.inner.state()
    }
}

#[async_trait::async_trait]
impl CallbackContext for StepTap {
    fn artifacts(&self) -> Option<Arc<dyn Artifacts>> {
        self.inner.artifacts()
    }
    fn tool_outcome(&self) -> Option<adk_core::ToolOutcome> {
        self.inner.tool_outcome()
    }
    fn tool_name(&self) -> Option<&str> {
        self.inner.tool_name()
    }
    fn tool_input(&self) -> Option<&serde_json::Value> {
        self.inner.tool_input()
    }
    fn shared_state(&self) -> Option<Arc<adk_core::SharedState>> {
        self.inner.shared_state()
    }
}

#[async_trait::async_trait]
impl ToolContext for StepTap {
    fn function_call_id(&self) -> &str {
        self.inner.function_call_id()
    }
    fn actions(&self) -> EventActions {
        self.inner.actions()
    }
    fn set_actions(&self, actions: EventActions) {
        self.inner.set_actions(actions);
    }
    async fn search_memory(&self, query: &str) -> adk_core::Result<Vec<MemoryEntry>> {
        self.inner.search_memory(query).await
    }
    fn memory(&self) -> Option<Arc<dyn Memory>> {
        self.inner.memory()
    }
    fn session(&self) -> Option<&dyn Session> {
        self.inner.session()
    }
    fn run_config(&self) -> Option<&RunConfig> {
        self.inner.run_config()
    }
    fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }
    fn request_metadata(&self) -> HashMap<String, serde_json::Value> {
        self.inner.request_metadata()
    }
    fn delegation_depth(&self) -> u32 {
        self.inner.delegation_depth()
    }
    fn max_delegation_depth(&self) -> Option<u32> {
        self.inner.max_delegation_depth()
    }
    fn orchestration_root_invocation_id(&self) -> &str {
        self.inner.orchestration_root_invocation_id()
    }
    fn orchestration_edge_id(&self) -> Option<&str> {
        self.inner.orchestration_edge_id()
    }
    async fn emit_event(&self, event: adk_core::Event) {
        for note in self.notes(&event) {
            let Ok(json) = serde_json::to_string(&note) else {
                continue;
            };
            let mut step = adk_core::Event::new(self.inner.invocation_id());
            step.author = note.helper;
            step.branch = self.inner.branch().to_owned();
            // Partial and content-free, like ADK's own tool-progress events:
            // shown to observers, never persisted, never part of model history.
            step.llm_response.partial = true;
            step.provider_metadata.insert(STEP_KEY.to_owned(), json);
            self.inner.emit_event(step).await;
        }
    }
    async fn emit_progress(&self, stream: &str, chunk: &str) {
        self.inner.emit_progress(stream, chunk).await;
    }
    fn user_scopes(&self) -> Vec<String> {
        self.inner.user_scopes()
    }
    async fn get_secret(&self, name: &str) -> adk_core::Result<Option<String>> {
        self.inner.get_secret(name).await
    }
    async fn get_secret_for_purpose(
        &self,
        name: &str,
        purpose: &str,
    ) -> adk_core::Result<Option<String>> {
        self.inner.get_secret_for_purpose(name, purpose).await
    }
}

/// Folds one run's ADK events into the live work log.
pub(crate) struct WorkTracker {
    activity: WorkActivity,
    started: Instant,
    message_id: String,
    /// Specialist id → "Web Search helper".
    helpers: HashMap<String, String>,
}

impl WorkTracker {
    pub(crate) fn new(message_id: String, started: Instant) -> Self {
        let helpers = sovra_core::default_sovra_srx_agents()
            .into_iter()
            .map(|spec| (spec.id, helper_display_name(&spec.name)))
            .collect();
        Self {
            activity: WorkActivity::default(),
            started,
            message_id,
            helpers,
        }
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Whether the log has anything to show; a plain answer shows no log.
    pub(crate) fn is_empty(&self) -> bool {
        self.activity.steps.is_empty() && self.activity.hidden_steps == 0
    }

    /// Applies one event; returns whether the log changed.
    pub(crate) fn observe(&mut self, event: &adk_core::Event) -> bool {
        let at_ms = self.now_ms();
        if let Some(note) = event.provider_metadata.get(STEP_KEY) {
            let Ok(note) = serde_json::from_str::<StepNote>(note) else {
                return false;
            };
            return self.apply_note(&note, at_ms);
        }
        let Some(content) = &event.llm_response.content else {
            return false;
        };
        let before = self.activity.clone();
        for (index, part) in content.parts.iter().enumerate() {
            match part {
                Part::FunctionCall { name, args, id, .. } => {
                    // Without an id a streamed chunk and the final event cannot be
                    // matched, so only the final event counts.
                    if id.is_none() && event.llm_response.partial {
                        continue;
                    }
                    let id = id.clone().unwrap_or_else(|| format!("{name}#{index}"));
                    let asks = self.helpers.get(name).cloned();
                    self.activity.start(StepCall {
                        id: &id,
                        helper: "",
                        helper_name: "",
                        tool: name,
                        args,
                        asks_helper: asks.as_deref(),
                        at_ms,
                    });
                }
                Part::FunctionResponse {
                    function_response,
                    id,
                    ..
                } => {
                    let id = id
                        .clone()
                        .unwrap_or_else(|| format!("{}#{index}", function_response.name));
                    self.activity
                        .finish_step(&id, !is_error(&function_response.response), at_ms);
                }
                _ => {}
            }
        }
        self.activity != before
    }

    fn apply_note(&mut self, note: &StepNote, at_ms: u64) -> bool {
        let before = self.activity.clone();
        match note.done {
            None => {
                let helper_name = self
                    .helpers
                    .get(&note.helper)
                    .cloned()
                    .unwrap_or_else(|| note.helper.clone());
                let asks = self.helpers.get(&note.tool).cloned();
                self.activity.start(StepCall {
                    id: &note.id,
                    helper: &note.lane,
                    helper_name: &helper_name,
                    tool: &note.tool,
                    args: &note.args,
                    asks_helper: asks.as_deref(),
                    at_ms,
                });
            }
            Some(ok) => self.activity.finish_step(&note.id, ok, at_ms),
        }
        self.activity != before
    }

    /// Closes the log: `ok` when the turn answered, otherwise it stopped early.
    pub(crate) fn finish(&mut self, ok: bool) {
        let at_ms = self.now_ms();
        self.activity.finish(ok, at_ms);
    }

    /// Sends the current log; `false` means the UI is gone.
    pub(crate) async fn send(&self, session_id: &str, events: &Sender<Event>) -> bool {
        events
            .send(Event::AgUi {
                session_id: session_id.to_owned(),
                event: AgUiEvent::ActivitySnapshot {
                    message_id: self.message_id.clone(),
                    activity: LiveActivity::Work(self.activity.clone()),
                },
            })
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use adk_core::{FunctionResponseData, LlmResponse};
    use serde_json::json;
    use sovra_core::work::StepState;

    use super::*;

    fn event(author: &str, parts: Vec<Part>, partial: bool) -> adk_core::Event {
        let mut event = adk_core::Event::new("inv");
        event.author = author.to_owned();
        event.llm_response = LlmResponse {
            content: Some(Content {
                role: "model".into(),
                parts,
            }),
            partial,
            ..LlmResponse::default()
        };
        event
    }

    fn call(name: &str, id: Option<&str>, args: serde_json::Value) -> Part {
        Part::FunctionCall {
            name: name.into(),
            args,
            id: id.map(str::to_owned),
            thought_signature: None,
        }
    }

    fn response(name: &str, id: Option<&str>, body: serde_json::Value) -> Part {
        Part::FunctionResponse {
            function_response: FunctionResponseData::new(name, body),
            id: id.map(str::to_owned),
            annotations: None,
        }
    }

    #[test]
    fn calls_and_results_become_steps_once() {
        let mut tracker = WorkTracker::new("work".into(), Instant::now());
        assert!(tracker.is_empty());
        let streamed = event(
            "OrchestratorAgent",
            vec![call("web_search", Some("c1"), json!({"query": "soup"}))],
            true,
        );
        assert!(tracker.observe(&streamed));
        assert!(
            !tracker.observe(&streamed),
            "the final copy changes nothing"
        );
        assert!(tracker.observe(&event(
            "OrchestratorAgent",
            vec![response("web_search", Some("c1"), json!({"results": []}))],
            false,
        )));
        let step = &tracker.activity.steps[0];
        assert_eq!(step.label, "Searching the web for “soup”");
        assert_eq!(step.state, StepState::Done);
    }

    #[test]
    fn delegations_open_helper_lanes_and_errors_fail_steps() {
        let mut tracker = WorkTracker::new("work".into(), Instant::now());
        tracker.observe(&event(
            "OrchestratorAgent",
            vec![
                call("WebSearchAgent", Some("d1"), json!({"request": "soup"})),
                call("CodingAgent", Some("d2"), json!({"request": "page"})),
            ],
            false,
        ));
        assert_eq!(tracker.activity.helpers.len(), 2);
        assert_eq!(tracker.activity.running().count(), 2);
        tracker.observe(&event(
            "OrchestratorAgent",
            vec![response(
                "CodingAgent",
                Some("d2"),
                json!({"error": "boom"}),
            )],
            false,
        ));
        assert_eq!(tracker.activity.helpers[1].state, StepState::Failed);
        assert_eq!(tracker.activity.helpers[0].name, "Web Search helper");
    }

    #[test]
    fn tapped_helper_steps_land_in_their_lane() {
        let mut tracker = WorkTracker::new("work".into(), Instant::now());
        let note = StepNote {
            lane: "d1".into(),
            helper: "WebSearchAgent".into(),
            id: "d1/c9".into(),
            tool: "web_search".into(),
            args: json!({"query": "leeks"}),
            done: None,
        };
        let mut tapped = adk_core::Event::new("inv");
        tapped
            .provider_metadata
            .insert(STEP_KEY.into(), serde_json::to_string(&note).unwrap());
        assert!(tracker.observe(&tapped));
        let lane = &tracker.activity.helpers[0];
        assert_eq!(lane.name, "Web Search helper");
        assert_eq!(lane.doing.as_deref(), Some("Searching the web for “leeks”"));
        tracker.finish(false);
        assert!(tracker.activity.stopped);
    }
}
