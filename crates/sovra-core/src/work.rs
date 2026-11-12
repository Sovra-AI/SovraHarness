//! The agent (`ReAct`) loop's rules and its live, plain-words progress log.
//!
//! An agent here is a model in a loop: it reasons, calls a tool, reads the
//! result, and repeats until it can answer. Running that loop is the adapter's
//! job (`sovra-adk`); this module owns the pure parts:
//!
//! - **Limits** every loop runs under: a step cap, a circuit breaker for a tool
//!   that keeps failing, and a guard against calling the same tool with the same
//!   arguments over and over (the most common way agents loop forever).
//! - **[`WorkActivity`]**, the AG-UI `ACTIVITY_SNAPSHOT` payload
//!   (`activityType: "sovra.work"`) that shows the person what is happening while
//!   it happens: each step in plain words, which helpers are working side by side,
//!   and how long each step took. Arguments are summarised, never dumped.
//!
//! Reasoning and sources: `docs/agent-loop.md`.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most model calls the top-level agent makes in one turn. A turn that needs
/// more is stopped with [`STEP_LIMIT_MESSAGE`] instead of running up the bill.
pub const PARENT_MAX_STEPS: u32 = 25;
/// Most model calls one specialist helper makes for one request.
pub const HELPER_MAX_STEPS: u32 = 25;
/// Most model calls one team member makes for one reply; members mostly talk.
pub const MEMBER_MAX_STEPS: u32 = 10;
/// Consecutive failures after which a tool is switched off for the rest of the
/// turn, so a broken connection cannot eat every remaining step.
pub const CIRCUIT_BREAKER_FAILURES: u32 = 3;
/// Identical calls (same tool, same arguments) allowed in one invocation.
/// The next identical call is refused with [`REPEATED_CALL_MESSAGE`].
pub const REPEAT_LIMIT: u32 = 3;
/// Steps kept in a snapshot; older finished ones are counted, not kept, so a long
/// turn never grows the event (or the saved chat) without bound.
pub const MAX_SHOWN_STEPS: usize = 60;
/// Longest step label, in characters.
const MAX_LABEL_CHARS: usize = 72;
/// Longest quoted argument inside a label, in characters.
const MAX_QUOTE_CHARS: usize = 48;
/// Invocations the repeat guard remembers at once (concurrent chats, helpers).
const REMEMBERED_INVOCATIONS: usize = 16;

/// Told to the model when it repeats itself, so it changes course instead.
pub const REPEATED_CALL_MESSAGE: &str = "You already made this exact call several times in this task and got the same result. Do not call it again with the same arguments: use the earlier result, change the arguments, try a different tool, or answer with what you have.";

/// Shown to the person when a turn hits [`PARENT_MAX_STEPS`].
pub const STEP_LIMIT_MESSAGE: &str = "I stopped after many steps so this wouldn't run on forever (and use up your AI credit). Ask me to carry on, or narrow the question, and I'll pick it up from here.";

/// True when an agent error is the loop's step cap rather than a real failure.
pub fn is_step_limit_error(message: &str) -> bool {
    message.contains("Max iterations")
}

/// What kind of thing a step does; picks the icon and the verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Search,
    Read,
    Write,
    Save,
    Run,
    Remember,
    Message,
    Plan,
    /// Handing part of the job to a specialist helper.
    Ask,
    Other,
}

/// Where a step (or helper) is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Running,
    Done,
    Failed,
}

/// One action in the loop: a tool call, or a request to a helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkStep {
    /// The tool call id; unique within the turn.
    pub id: String,
    /// The helper doing it; empty for Sovra itself.
    #[serde(default)]
    pub helper: String,
    /// For an [`StepKind::Ask`] step, the helper lane it opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub kind: StepKind,
    /// Plain words: "Searching the web for “soup recipes”".
    pub label: String,
    pub state: StepState,
    /// Milliseconds from the start of the turn.
    pub started_ms: u64,
    #[serde(default)]
    pub finished_ms: Option<u64>,
}

impl WorkStep {
    /// How long the step took, once it finished.
    pub fn duration_ms(&self) -> Option<u64> {
        self.finished_ms
            .map(|end| end.saturating_sub(self.started_ms))
    }
}

/// A specialist helper working on part of the turn; helpers run side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkHelper {
    pub id: String,
    pub name: String,
    pub state: StepState,
    /// What it is doing right now, in plain words.
    #[serde(default)]
    pub doing: Option<String>,
    /// Steps it has taken so far.
    #[serde(default)]
    pub steps: u32,
}

/// The live work log for one turn (`activityType: "sovra.work"`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkActivity {
    /// One plain sentence: what is happening now, or how it ended.
    pub status: String,
    pub steps: Vec<WorkStep>,
    pub helpers: Vec<WorkHelper>,
    pub finished: bool,
    /// Whether the turn ended early (failure or step limit).
    #[serde(default)]
    pub stopped: bool,
    /// Finished steps dropped from `steps` to keep the snapshot small.
    #[serde(default)]
    pub hidden_steps: u32,
    /// Milliseconds from the start of the turn to the latest change.
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// A tool call the loop just made, as the adapter saw it.
#[derive(Debug, Clone, Copy)]
pub struct StepCall<'a> {
    pub id: &'a str,
    /// The helper lane making the call (the id of the request that asked it);
    /// empty for Sovra itself.
    pub helper: &'a str,
    /// How to show that helper; ignored when `helper` is empty.
    pub helper_name: &'a str,
    pub tool: &'a str,
    pub args: &'a Value,
    /// Set when the call hands work to a specialist helper: its display name.
    pub asks_helper: Option<&'a str>,
    pub at_ms: u64,
}

impl WorkActivity {
    /// Records a new step. Repeated reports of the same call id are ignored, so
    /// adapters may report from both streamed and final events.
    pub fn start(&mut self, call: StepCall<'_>) {
        self.elapsed_ms = self.elapsed_ms.max(call.at_ms);
        if self.steps.iter().any(|step| step.id == call.id) {
            return;
        }
        let (kind, label, target) = if let Some(name) = call.asks_helper {
            // One lane per request, not per helper: the same helper asked two
            // things at once shows as two lanes working side by side.
            let lane = self.helper_mut(call.id, name);
            lane.state = StepState::Running;
            lane.doing = Some("Getting started".to_owned());
            (
                StepKind::Ask,
                ask_label(name, call.args),
                Some(call.id.to_owned()),
            )
        } else {
            let (kind, label) = describe_step(call.tool, call.args);
            (kind, label, None)
        };
        if !call.helper.is_empty() {
            let name = if call.helper_name.is_empty() {
                call.helper
            } else {
                call.helper_name
            };
            let lane = self.helper_mut(call.helper, name);
            lane.state = StepState::Running;
            lane.steps = lane.steps.saturating_add(1);
            lane.doing = Some(label.clone());
        }
        self.steps.push(WorkStep {
            id: call.id.to_owned(),
            helper: call.helper.to_owned(),
            target,
            kind,
            label,
            state: StepState::Running,
            started_ms: call.at_ms,
            finished_ms: None,
        });
        self.trim();
        self.refresh_status();
    }

    /// Marks a step finished. Unknown or already-finished ids are ignored.
    pub fn finish_step(&mut self, id: &str, ok: bool, at_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.max(at_ms);
        let Some(step) = self
            .steps
            .iter_mut()
            .find(|step| step.id == id && step.state == StepState::Running)
        else {
            return;
        };
        step.state = settled(ok);
        step.finished_ms = Some(at_ms);
        let (target, helper_id) = (step.target.clone(), step.helper.clone());
        if let Some(target) = target
            && let Some(lane) = self.helpers.iter_mut().find(|lane| lane.id == target)
        {
            lane.state = settled(ok);
            lane.doing = None;
        }
        if !helper_id.is_empty() {
            let doing = self
                .steps
                .iter()
                .rev()
                .find(|step| step.helper == helper_id && step.state == StepState::Running)
                .map(|step| step.label.clone());
            if let Some(lane) = self.helpers.iter_mut().find(|lane| lane.id == helper_id) {
                lane.doing = doing;
            }
        }
        self.refresh_status();
    }

    /// Ends the log. Anything still running is settled: done when the turn
    /// succeeded, failed when it stopped early.
    pub fn finish(&mut self, ok: bool, at_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.max(at_ms);
        for step in &mut self.steps {
            if step.state == StepState::Running {
                step.state = settled(ok);
                step.finished_ms = Some(at_ms);
            }
        }
        for lane in &mut self.helpers {
            if lane.state == StepState::Running {
                lane.state = settled(ok);
            }
            lane.doing = None;
        }
        self.finished = true;
        self.stopped = !ok;
        self.status = self.summary();
    }

    /// Steps running right now; more than one means work is happening in parallel.
    pub fn running(&self) -> impl Iterator<Item = &WorkStep> {
        self.steps
            .iter()
            .filter(|step| step.state == StepState::Running)
    }

    /// All steps taken, including ones trimmed from the snapshot.
    pub fn total_steps(&self) -> u32 {
        self.hidden_steps
            .saturating_add(u32::try_from(self.steps.len()).unwrap_or(u32::MAX))
    }

    /// "Worked through 4 steps with 2 helpers in 12s" — the closed log's title.
    pub fn summary(&self) -> String {
        let steps = self.total_steps();
        let noun = if steps == 1 { "step" } else { "steps" };
        let verb = if self.stopped {
            "Stopped after"
        } else {
            "Worked through"
        };
        let mut text = format!("{verb} {steps} {noun}");
        match self.helpers.len() {
            0 => {}
            1 => text.push_str(" with 1 helper"),
            many => {
                let _ = write!(text, " with {many} helpers");
            }
        }
        if self.elapsed_ms >= 1_000 {
            text.push_str(" in ");
            text.push_str(&format_duration(self.elapsed_ms));
        }
        text
    }

    fn helper_mut(&mut self, id: &str, name: &str) -> &mut WorkHelper {
        let index = if let Some(index) = self.helpers.iter().position(|lane| lane.id == id) {
            index
        } else {
            self.helpers.push(WorkHelper {
                id: id.to_owned(),
                name: name.to_owned(),
                state: StepState::Running,
                doing: None,
                steps: 0,
            });
            self.helpers.len() - 1
        };
        &mut self.helpers[index]
    }

    fn trim(&mut self) {
        while self.steps.len() > MAX_SHOWN_STEPS {
            let Some(index) = self
                .steps
                .iter()
                .position(|step| step.state != StepState::Running)
            else {
                return;
            };
            self.steps.remove(index);
            self.hidden_steps = self.hidden_steps.saturating_add(1);
        }
    }

    fn refresh_status(&mut self) {
        let running: Vec<&WorkStep> = self.running().collect();
        self.status = match running.as_slice() {
            [] => "Working out what to do next…".to_owned(),
            [only] => format!("{}…", only.label),
            [.., latest] => format!("Doing {} things at once — {}…", running.len(), latest.label),
        };
    }
}

fn settled(ok: bool) -> StepState {
    if ok {
        StepState::Done
    } else {
        StepState::Failed
    }
}

/// "1.2s", "45s", "2m 05s".
pub fn format_duration(ms: u64) -> String {
    if ms < 10_000 {
        let tenths = ms / 100;
        format!("{}.{}s", tenths / 10, tenths % 10)
    } else if ms < 60_000 {
        format!("{}s", ms / 1_000)
    } else {
        let seconds = ms / 1_000;
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    }
}

/// "Web Search Specialist" → "Web Search helper": the people using Sovra say
/// "helper", never "agent" or "specialist".
pub fn helper_display_name(specialist_name: &str) -> String {
    let base = ["Specialist", "Agent", "Pipeline", "Helper"]
        .iter()
        .find_map(|suffix| specialist_name.trim().strip_suffix(suffix))
        .unwrap_or(specialist_name)
        .trim();
    if base.is_empty() {
        "Helper".to_owned()
    } else {
        format!("{base} helper")
    }
}

fn ask_label(helper: &str, args: &Value) -> String {
    match quoted_arg(args, &["request", "task", "query", "prompt"]) {
        Some(quote) => clip(&format!("Asking the {helper} to look at “{quote}”")),
        None => clip(&format!("Asking the {helper}")),
    }
}

/// Plain words for a tool call. Known tools get a sentence; anything else (an
/// MCP server's tool, a new integration) gets its name made readable, so a new
/// tool is never shown as raw JSON.
pub fn describe_step(tool: &str, args: &Value) -> (StepKind, String) {
    let quote = |keys: &[&str]| quoted_arg(args, keys);
    let with = |text: &str, keys: &[&str]| match quote(keys) {
        Some(q) => format!("{text} “{q}”"),
        None => text.to_owned(),
    };
    let (kind, label) = match tool {
        "web_search" => (
            StepKind::Search,
            with("Searching the web for", &["query", "q"]),
        ),
        "read_web_page" => (StepKind::Read, with("Reading", &["url", "address"])),
        "search_saved_details" | "query_file_knowledge" | "read_file_knowledge_evidence" => (
            StepKind::Remember,
            with(
                "Checking what I remember about",
                &["query", "q", "question"],
            ),
        ),
        "search_indexed_files" | "query_indexed_files" | "byte_search" | "search_attachments" => (
            StepKind::Search,
            with("Searching your files for", &["query", "q", "pattern"]),
        ),
        "read_indexed_file"
        | "read_private_file"
        | "read_attachment"
        | "read_file_history"
        | "inspect_file_history"
        | "read_drive_file" => (
            StepKind::Read,
            with("Reading", &["file_name", "path", "name", "title"]),
        ),
        "list_attachments" | "list_indexed_folders" => {
            (StepKind::Read, "Looking through your files".to_owned())
        }
        "search_email" => (
            StepKind::Search,
            with("Searching your email for", &["query", "q"]),
        ),
        "read_email" => (StepKind::Read, "Reading an email".to_owned()),
        "draft_email" => (
            StepKind::Write,
            with("Drafting an email to", &["to", "recipient"]),
        ),
        "search_drive_files" => (
            StepKind::Search,
            with("Searching Drive for", &["query", "q"]),
        ),
        "search_contacts" | "find_people" => (
            StepKind::Search,
            with("Looking up", &["query", "name", "q"]),
        ),
        "list_calendar_events" => (StepKind::Read, "Checking your calendar".to_owned()),
        "add_calendar_event" => (
            StepKind::Write,
            with("Adding to your calendar:", &["title", "summary"]),
        ),
        "list_todos" => (StepKind::Read, "Checking your to-do list".to_owned()),
        "add_todo" => (StepKind::Write, with("Adding a to-do:", &["title", "text"])),
        "save_artifact" => (StepKind::Save, with("Saving", &["title"])),
        "deploy_artifact" => (StepKind::Run, "Starting your website".to_owned()),
        "run_terminal_command" => (StepKind::Run, with("Running", &["command"])),
        "load_skill" => (
            StepKind::Plan,
            with("Opening the skill", &["name", "skill"]),
        ),
        "create_schedule" | "list_schedules" | "run_schedule_now" => {
            (StepKind::Plan, "Working on your schedules".to_owned())
        }
        "send_text" | "prepare_message" | "send_channel" | "read_recent_messages" => {
            (StepKind::Message, "Working on a message".to_owned())
        }
        "propose_file_knowledge" => (StepKind::Remember, "Noting what your files say".to_owned()),
        "list_recent_chats" => (StepKind::Read, "Looking back at recent chats".to_owned()),
        other => (StepKind::Other, readable_tool_name(other)),
    };
    (kind, clip(&label))
}

/// `render_table` → "Using render table": an unknown tool still reads as words.
fn readable_tool_name(tool: &str) -> String {
    let words: Vec<String> = tool
        .split(['_', '.', '-', '/'])
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let text = words.join(" ");
    if text.is_empty() {
        "Using a tool".to_owned()
    } else {
        format!("Using {text}")
    }
}

fn quoted_arg(args: &Value, keys: &[&str]) -> Option<String> {
    let object = args.as_object()?;
    let value = keys.iter().find_map(|key| object.get(*key))?;
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Value::Number(number) => number.to_string(),
        Value::Null | Value::Bool(_) | Value::Object(_) => return None,
    };
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!flat.is_empty()).then(|| clip_to(&flat, MAX_QUOTE_CHARS))
}

fn clip(text: &str) -> String {
    clip_to(text, MAX_LABEL_CHARS)
}

fn clip_to(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.truncate(clipped.trim_end().len());
    clipped.push('…');
    clipped
}

/// Counts identical tool calls per invocation so a looping agent is told to
/// change course. Bounded: only the most recent invocations are remembered.
#[derive(Debug, Default)]
pub struct RepeatGuard {
    invocations: VecDeque<(String, HashMap<String, u32>)>,
}

impl RepeatGuard {
    /// Records a call; returns `false` when it exceeds [`REPEAT_LIMIT`] and should
    /// be refused with [`REPEATED_CALL_MESSAGE`].
    pub fn allow(&mut self, invocation: &str, tool: &str, args: &Value) -> bool {
        let key = format!("{tool}\u{1f}{}", canonical_json(args));
        let index =
            if let Some(index) = self.invocations.iter().position(|(id, _)| id == invocation) {
                index
            } else {
                if self.invocations.len() >= REMEMBERED_INVOCATIONS {
                    self.invocations.pop_front();
                }
                self.invocations
                    .push_back((invocation.to_owned(), HashMap::new()));
                self.invocations.len() - 1
            };
        let count = self.invocations[index].1.entry(key).or_insert(0);
        *count = count.saturating_add(1);
        *count <= REPEAT_LIMIT
    }
}

/// JSON with object keys sorted, so `{"a":1,"b":2}` and `{"b":2,"a":1}` match.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{key:?}:{}", canonical_json(&object[key])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn call<'a>(id: &'a str, tool: &'a str, args: &'a Value, at_ms: u64) -> StepCall<'a> {
        StepCall {
            id,
            helper: "",
            helper_name: "",
            tool,
            args,
            asks_helper: None,
            at_ms,
        }
    }

    #[test]
    fn known_tools_read_as_plain_sentences() {
        let (kind, label) = describe_step("web_search", &json!({"query": "best soup"}));
        assert_eq!(kind, StepKind::Search);
        assert_eq!(label, "Searching the web for “best soup”");
        let (_, label) = describe_step(
            "run_terminal_command",
            &json!({"command": "npm  run\nbuild"}),
        );
        assert_eq!(label, "Running “npm run build”");
        let (kind, label) = describe_step("save_artifact", &json!({}));
        assert_eq!((kind, label.as_str()), (StepKind::Save, "Saving"));
    }

    #[test]
    fn unknown_tools_get_readable_names_not_json() {
        let (kind, label) = describe_step("GoogleCalendar.list_events", &json!({"x": 1}));
        assert_eq!(kind, StepKind::Other);
        assert_eq!(label, "Using googlecalendar list events");
        assert_eq!(describe_step("", &json!(null)).1, "Using a tool");
    }

    #[test]
    fn long_arguments_are_clipped_on_character_boundaries() {
        let long = "茶".repeat(200);
        let (_, label) = describe_step("web_search", &json!({ "query": long }));
        assert!(label.chars().count() <= MAX_LABEL_CHARS);
        assert!(label.ends_with('…') || label.ends_with("…”"));
    }

    #[test]
    fn helper_names_say_helper() {
        assert_eq!(
            helper_display_name("Web Search Specialist"),
            "Web Search helper"
        );
        assert_eq!(
            helper_display_name("Deep Research Pipeline"),
            "Deep Research helper"
        );
        assert_eq!(helper_display_name("Agent"), "Helper");
    }

    #[test]
    fn steps_start_once_and_finish_with_durations() {
        let mut log = WorkActivity::default();
        let args = json!({"query": "soup"});
        log.start(call("c1", "web_search", &args, 100));
        log.start(call("c1", "web_search", &args, 150));
        assert_eq!(log.steps.len(), 1, "a re-reported call is one step");
        assert_eq!(log.status, "Searching the web for “soup”…");
        log.finish_step("c1", true, 1_300);
        assert_eq!(log.steps[0].state, StepState::Done);
        assert_eq!(log.steps[0].duration_ms(), Some(1_200));
        assert_eq!(log.status, "Working out what to do next…");
        log.finish(true, 2_500);
        assert!(log.finished && !log.stopped);
        assert_eq!(log.status, "Worked through 1 step in 2.5s");
    }

    #[test]
    fn parallel_helpers_get_their_own_lanes() {
        let mut log = WorkActivity::default();
        let request = json!({"request": "find a recipe"});
        let ask = |id, tool, name| StepCall {
            asks_helper: Some(name),
            ..call(id, tool, &request, 0)
        };
        log.start(ask("a", "WebSearchAgent", "Web Search helper"));
        log.start(ask("b", "CodingAgent", "Coding helper"));
        assert_eq!(log.running().count(), 2);
        assert!(log.status.starts_with("Doing 2 things at once"));
        let query = json!({"query": "soup"});
        log.start(StepCall {
            helper: "a",
            helper_name: "Web Search helper",
            ..call("a.1", "web_search", &query, 10)
        });
        let lane = &log.helpers[0];
        assert_eq!(lane.steps, 1);
        assert_eq!(lane.doing.as_deref(), Some("Searching the web for “soup”"));
        log.finish_step("a.1", true, 20);
        assert_eq!(log.helpers[0].doing, None);
        log.finish_step("a", true, 30);
        log.finish_step("b", false, 40);
        assert_eq!(log.helpers[0].state, StepState::Done);
        assert_eq!(log.helpers[1].state, StepState::Failed);
        assert_eq!(
            log.steps[0].label,
            "Asking the Web Search helper to look at “find a recipe”"
        );
    }

    #[test]
    fn stopping_early_settles_running_work_as_failed() {
        let mut log = WorkActivity::default();
        let args = json!({});
        log.start(call("c1", "web_search", &args, 0));
        log.finish(false, 5_000);
        assert_eq!(log.steps[0].state, StepState::Failed);
        assert_eq!(log.status, "Stopped after 1 step in 5.0s");
    }

    #[test]
    fn the_snapshot_stays_bounded() {
        let mut log = WorkActivity::default();
        let args = json!({});
        for n in 0..(MAX_SHOWN_STEPS + 15) {
            let id = format!("c{n}");
            log.start(call(&id, "web_search", &args, 0));
            log.finish_step(&id, true, 1);
        }
        assert_eq!(log.steps.len(), MAX_SHOWN_STEPS);
        assert_eq!(
            log.total_steps(),
            u32::try_from(MAX_SHOWN_STEPS + 15).unwrap()
        );
    }

    #[test]
    fn repeat_guard_refuses_the_same_call_after_the_limit() {
        let mut guard = RepeatGuard::default();
        let a = json!({"query": "soup", "page": 1});
        let same = json!({"page": 1, "query": "soup"});
        for _ in 0..REPEAT_LIMIT {
            assert!(guard.allow("inv", "web_search", &a));
        }
        assert!(
            !guard.allow("inv", "web_search", &same),
            "key order does not matter"
        );
        assert!(guard.allow("inv", "web_search", &json!({"query": "stew"})));
        assert!(
            guard.allow("other", "web_search", &a),
            "a new turn starts fresh"
        );
    }

    #[test]
    fn repeat_guard_forgets_old_invocations() {
        let mut guard = RepeatGuard::default();
        for n in 0..(REMEMBERED_INVOCATIONS * 3) {
            guard.allow(&n.to_string(), "t", &json!({}));
        }
        assert_eq!(guard.invocations.len(), REMEMBERED_INVOCATIONS);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(0), "0.0s");
        assert_eq!(format_duration(1_250), "1.2s");
        assert_eq!(format_duration(42_000), "42s");
        assert_eq!(format_duration(125_000), "2m 05s");
    }

    #[test]
    fn step_limit_errors_are_recognised() {
        assert!(is_step_limit_error("agent: Max iterations (25) exceeded"));
        assert!(!is_step_limit_error("network down"));
    }
}
