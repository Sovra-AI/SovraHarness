//! `/coding` file-writing run.
//!
//! The shortcut starts this loop instead of a chat specialist. The model writes
//! relative files with tools, then the run saves them in the library.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use adk_agent::LlmAgentBuilder;
use adk_core::{Content, EventTextDeltas, Tool, ToolContext};
use adk_runner::Runner;
use adk_session::{CreateRequest, InMemorySessionService, SessionService};
use async_channel::Sender;
use futures::StreamExt as _;
use serde_json::{Value, json};
use sovra_core::artifacts::{ArtifactDraft, ArtifactKind, ArtifactSource};
use sovra_core::{
    AgUiEvent, AgUiWidget, CodeWorkspace, Event, LiveActivity, SavedCode, language_for_path,
    package_code,
};

use crate::{AdkHarness, DEFAULT_USER_ID, extract_output};

const PREVIEW_CHARS: usize = 8_000;

const INSTRUCTION: &str = "\
You write files when someone asks for something to be built or changed. The person is not a programmer.

Use write_file, read_file, edit_file and list_files. Paths are relative, such as index.html or src/App.tsx.

- For a website, flyer or small browser page, write one complete index.html. Put the styles and any script inside that file. Use the person's real details, not placeholder text.
- For React, Vue, Svelte or another framework, write every source file plus package.json. package.json must have a scripts.build command that produces static files.
- For anything else, write the source files the request needs.
- Never use an absolute path, .., or a folder named node_modules or .git.
- When the files are complete, stop calling tools. Reply with one short sentence the person can understand. Do not paste the source into the reply.
";

/// One `/coding` turn handed over by the runtime.
#[derive(Debug, Clone)]
pub struct OpenCodeTurn {
    pub session_id: String,
    pub run_id: String,
    pub prompt: String,
}

struct CodeRun<'a> {
    harness: &'a AdkHarness,
    turn: &'a OpenCodeTurn,
    events: &'a Sender<Event>,
    workspace: Arc<Mutex<CodeWorkspace>>,
    activity_id: String,
    deployment: Option<Arc<dyn sovra_core::raspberry_pi::PiDeployment>>,
}

impl AdkHarness {
    /// Write the requested files and save them. Streams a code preview, a short
    /// closing note and the saved-work cards.
    pub async fn run_opencode(&self, turn: &OpenCodeTurn, events: &Sender<Event>) {
        self.run_opencode_for_pi(turn, events, None).await;
    }

    /// The same file-writing run, with delivery to the explicitly selected Pi.
    pub async fn run_opencode_for_pi(
        &self,
        turn: &OpenCodeTurn,
        events: &Sender<Event>,
        deployment: Option<Arc<dyn sovra_core::raspberry_pi::PiDeployment>>,
    ) {
        let run = CodeRun {
            harness: self,
            turn,
            events,
            workspace: Arc::new(Mutex::new(CodeWorkspace::default())),
            activity_id: format!("{}-code", turn.run_id),
            deployment,
        };
        if !run
            .send(AgUiEvent::RunStarted {
                run_id: turn.run_id.clone(),
            })
            .await
        {
            return;
        }
        if let Some(target) = &run.deployment
            && let Err(message) = target.check().await
        {
            run.fail(&message).await;
            return;
        }
        let tools = run.tools();
        let reply = match run.collect(&tools).await {
            Some(Ok(text)) => text,
            Some(Err(message)) => {
                run.fail(&message).await;
                return;
            }
            None => return,
        };
        let workspace = run
            .workspace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let pieces = match package_code(&workspace, &turn.prompt) {
            Ok(pieces) => pieces,
            Err(message) => {
                run.fail(&friendly_empty(&message)).await;
                return;
            }
        };
        let mut cards = Vec::new();
        let mut failures = Vec::new();
        for piece in &pieces {
            match run.save(piece).await {
                Some(Ok((id, title, kind))) => cards.push((id, title, kind)),
                Some(Err(message)) => failures.push(message),
                None => return,
            }
        }
        let mut note = closing_note(&reply, &pieces);
        if failures.is_empty()
            && let Some(target) = &run.deployment
        {
            let project = sovra_core::raspberry_pi::board(target.board_id())
                .ok_or_else(|| "Choose a supported Pi model.".to_owned())
                .and_then(|board| {
                    sovra_core::raspberry_pi::PiProject::from_workspace(&workspace, board)
                });
            let outcome = match project {
                Ok(project) => target.deploy(project).await,
                Err(error) => Err(error),
            };
            note.push_str("\n\n");
            match outcome {
                Ok(receipt) => note.push_str(&receipt),
                Err(problem) => {
                    note.push_str("Your code is saved, but deployment did not finish: ");
                    note.push_str(&problem);
                }
            }
        }
        if !failures.is_empty() {
            note.push_str(" Some of it could not be saved: ");
            note.push_str(&failures.join(" "));
        }
        if !run.send_closing(&note, &cards).await {
            return;
        }
        let _ = run
            .send(AgUiEvent::RunFinished {
                run_id: turn.run_id.clone(),
            })
            .await;
    }
}

impl CodeRun<'_> {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        ["list_files", "read_file", "write_file", "edit_file"]
            .into_iter()
            .map(|name| {
                Arc::new(CodeTool {
                    name,
                    workspace: Arc::clone(&self.workspace),
                    events: self.events.clone(),
                    session_id: self.turn.session_id.clone(),
                    activity_id: self.activity_id.clone(),
                }) as Arc<dyn Tool>
            })
            .collect()
    }

    async fn collect(&self, tools: &[Arc<dyn Tool>]) -> Option<Result<String, String>> {
        let invocation = self.turn.run_id.clone();
        let runner = match self.harness.coding_runner(&invocation, tools).await {
            Ok(runner) => runner,
            Err(error) => return Some(Err(self.harness.safe_error(&error))),
        };
        let mut prompt = format!(
            "The person asked for this to be built or changed:\n\n{}",
            self.turn.prompt
        );
        if let Some(board) = self
            .deployment
            .as_ref()
            .and_then(|target| sovra_core::raspberry_pi::board(target.board_id()))
        {
            use std::fmt::Write as _;
            let _ = write!(
                prompt,
                "\n\nTarget: {}. This request will deploy automatically after the files are saved. Write main.py (MicroPython on Pico, Python on Pi computers), main.js (Node on computers), or index.html (static website on computers). Use standard libraries, built-in web APIs or gpiozero already on the computer. No dependency installation or build step is available. A website serves on port 8080. Do not change OS settings or unrelated files. Compute Module GPIO needs the exact carrier; ask first if missing. GPIO reference: {}",
                board.name,
                board.gpio_guide()
            );
        }
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

    async fn save(
        &self,
        piece: &SavedCode,
    ) -> Option<Result<(String, String, ArtifactKind), String>> {
        let Some(saver) = &self.harness.artifact_saver else {
            return Some(Err("Saving work is unavailable in this session.".into()));
        };
        let draft = ArtifactDraft {
            title: piece.title.clone(),
            kind: piece.kind,
            language: piece.language.clone(),
            content: piece.content.clone(),
            source: ArtifactSource {
                session_id: self.turn.session_id.clone(),
                message_id: self.activity_id.clone(),
                author: "Sovra".into(),
                shared_from: String::new(),
            },
        };
        let title = piece.title.clone();
        let kind = piece.kind;
        match saver.save(draft).await {
            Ok(artifact) => {
                if !self
                    .send(AgUiEvent::ActivitySnapshot {
                        message_id: self.activity_id.clone(),
                        activity: LiveActivity::CodeWriting {
                            language: piece.language.clone(),
                            content: piece.content.chars().take(PREVIEW_CHARS).collect(),
                            artifact_id: Some(artifact.id.clone()),
                        },
                    })
                    .await
                {
                    return None;
                }
                Some(Ok((artifact.id, title, kind)))
            }
            Err(message) => Some(Err(message)),
        }
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

    async fn send_closing(&self, text: &str, cards: &[(String, String, ArtifactKind)]) -> bool {
        let message_id = format!("{}-closing", self.turn.run_id);
        if !self
            .send(AgUiEvent::TextMessageStart {
                message_id: message_id.clone(),
                subagent_run_id: None,
            })
            .await
            || !self
                .send(AgUiEvent::TextMessageContent {
                    message_id: message_id.clone(),
                    delta: text.into(),
                })
                .await
            || !self.send(AgUiEvent::TextMessageEnd { message_id }).await
        {
            return false;
        }
        for (artifact_id, title, artifact_kind) in cards {
            if self
                .events
                .send(Event::AgUiSurface {
                    session_id: self.turn.session_id.clone(),
                    widget: AgUiWidget::ArtifactSaved {
                        artifact_id: artifact_id.clone(),
                        title: title.clone(),
                        artifact_kind: *artifact_kind,
                    },
                })
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    async fn fail(&self, message: &str) {
        let _ = self
            .send(AgUiEvent::RunError {
                message: message.into(),
            })
            .await;
    }
}

impl AdkHarness {
    async fn coding_runner(
        &self,
        invocation: &str,
        tools: &[Arc<dyn Tool>],
    ) -> Result<Runner, crate::Error> {
        let mut builder = crate::work_log::harden(
            LlmAgentBuilder::new("Coding")
                .description("Writes files for a build or change request and saves them.")
                .instruction(INSTRUCTION)
                .global_instruction(&self.standing_instruction)
                .model(Arc::clone(&self.llm)),
            sovra_core::work::HELPER_MAX_STEPS,
        );
        for tool in tools {
            builder = builder.tool(Arc::clone(tool));
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

struct CodeTool {
    name: &'static str,
    workspace: Arc<Mutex<CodeWorkspace>>,
    events: Sender<Event>,
    session_id: String,
    activity_id: String,
}

impl CodeTool {
    fn locked<T>(&self, body: impl FnOnce(&mut CodeWorkspace) -> T) -> T {
        let mut workspace = self
            .workspace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        body(&mut workspace)
    }
}

#[async_trait::async_trait]
impl Tool for CodeTool {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        match self.name {
            "list_files" => "List the relative file names written in this coding run.",
            "read_file" => "Read one relative file written in this coding run.",
            "write_file" => "Create or replace one relative file with its complete contents.",
            "edit_file" => "Replace one exact occurrence of old text in a relative file.",
            _ => "Coding file tool.",
        }
    }

    fn is_read_only(&self) -> bool {
        matches!(self.name, "list_files" | "read_file")
    }

    fn parameters_schema(&self) -> Option<Value> {
        let path = json!({"type": "string", "description": "Relative file name, such as index.html or src/App.tsx."});
        Some(match self.name {
            "list_files" => {
                json!({"type": "object", "properties": {}, "additionalProperties": false})
            }
            "read_file" => {
                json!({"type": "object", "properties": {"path": path}, "required": ["path"], "additionalProperties": false})
            }
            "write_file" => json!({
                "type": "object",
                "properties": {
                    "path": path,
                    "content": {"type": "string", "description": "The complete file contents."}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
            _ => json!({
                "type": "object",
                "properties": {
                    "path": path,
                    "old": {"type": "string", "description": "Exact text to replace. It must appear once."},
                    "new": {"type": "string", "description": "Text to put in its place."}
                },
                "required": ["path", "old", "new"],
                "additionalProperties": false
            }),
        })
    }

    async fn execute(&self, _: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        if !args.is_object() && self.name != "list_files" {
            return Ok(json!({"error": "Arguments must be an object."}));
        }
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let outcome = match self.name {
            "list_files" => Ok(json!({"files": self.locked(|workspace| workspace.list())})),
            "read_file" => self
                .locked(|workspace| workspace.read(&text("path")))
                .map(|content| json!({"path": text("path"), "content": content})),
            "write_file" => {
                let path = text("path");
                let content = text("content");
                self.locked(|workspace| workspace.write(&path, &content))
                    .map(|()| json!({"path": path, "bytes": content.len()}))
            }
            "edit_file" => {
                let path = text("path");
                self.locked(|workspace| workspace.edit(&path, &text("old"), &text("new")))
                    .map(|()| json!({"path": path, "updated": true}))
            }
            _ => Err("This coding tool is not available.".into()),
        };
        match outcome {
            Ok(value) => {
                if matches!(self.name, "write_file" | "edit_file") {
                    let path = text("path");
                    let preview = self.locked(|workspace| {
                        workspace.read(&path).ok().map(|content| {
                            (
                                language_for_path(&path).to_owned(),
                                content.chars().take(PREVIEW_CHARS).collect::<String>(),
                            )
                        })
                    });
                    if let Some((language, content)) = preview {
                        let _ = self
                            .events
                            .send(Event::AgUi {
                                session_id: self.session_id.clone(),
                                event: AgUiEvent::ActivitySnapshot {
                                    message_id: self.activity_id.clone(),
                                    activity: LiveActivity::CodeWriting {
                                        language,
                                        content,
                                        artifact_id: None,
                                    },
                                },
                            })
                            .await;
                    }
                }
                Ok(value)
            }
            Err(message) => Ok(json!({"error": message})),
        }
    }
}

fn friendly_empty(message: &str) -> String {
    if message == "No files were written." {
        "I didn't write any files for that. Say what you want built, like a page or a small program.".into()
    } else {
        message.to_owned()
    }
}

fn closing_note(model_text: &str, saved: &[SavedCode]) -> String {
    let trimmed = model_text.trim();
    let plain = !trimmed.is_empty()
        && trimmed.chars().count() <= 400
        && !trimmed.contains("```")
        && !trimmed.to_ascii_lowercase().contains("<html");
    if plain {
        return trimmed.to_owned();
    }
    match saved {
        [one] => format!("Saved “{}”.", one.title),
        many => format!("Saved {} things in your library.", many.len()),
    }
}

/// Unique-ish run id for tests and one-shot launches.
pub fn new_opencode_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("coding-{millis}")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use adk_core::{FinishReason, Llm, LlmRequest, LlmResponse, LlmResponseStream, Part};
    use sovra_core::artifacts::Artifact;
    use sovra_core::{AgentConfig, Timestamp};

    use super::{OpenCodeTurn, new_opencode_run_id};
    use crate::AdkHarness;
    use crate::artifacts::ArtifactSaver;

    struct ScriptLlm {
        calls: AtomicUsize,
        pages: Vec<serde_json::Value>,
    }

    #[async_trait::async_trait]
    impl Llm for ScriptLlm {
        fn name(&self) -> &'static str {
            "coding-test"
        }

        async fn generate_content(
            &self,
            _request: LlmRequest,
            _stream: bool,
        ) -> adk_core::Result<LlmResponseStream> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let response = if let Some(args) = self.pages.get(call) {
                LlmResponse {
                    content: Some(adk_core::Content {
                        role: "model".into(),
                        parts: vec![Part::FunctionCall {
                            name: "write_file".into(),
                            args: args.clone(),
                            id: Some(format!("write-{call}")),
                            thought_signature: None,
                        }],
                    }),
                    finish_reason: Some(FinishReason::Stop),
                    partial: false,
                    turn_complete: true,
                    ..LlmResponse::default()
                }
            } else {
                LlmResponse {
                    content: Some(
                        adk_core::Content::new("model")
                            .with_text("Your garden club page is ready to open."),
                    ),
                    finish_reason: Some(FinishReason::Stop),
                    partial: false,
                    turn_complete: true,
                    ..LlmResponse::default()
                }
            };
            Ok(Box::pin(futures::stream::iter(vec![Ok(response)])))
        }
    }

    fn page(path: &str, content: &str) -> serde_json::Value {
        serde_json::json!({"path": path, "content": content})
    }

    async fn run(pages: Vec<serde_json::Value>) -> (Vec<Event>, Vec<ArtifactDraft>) {
        let llm = std::sync::Arc::new(ScriptLlm {
            calls: AtomicUsize::new(0),
            pages,
        });
        let base =
            AdkHarness::with_llm_and_tools(&AgentConfig::default(), llm, vec![]).expect("harness");
        let drafts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let saved = std::sync::Arc::clone(&drafts);
        let harness = base
            .with_artifact_saver(ArtifactSaver::from_fn(move |draft| {
                let saved = std::sync::Arc::clone(&saved);
                async move {
                    saved
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(draft.clone());
                    Ok(Artifact::from_draft("code-1".into(), draft, Timestamp(1)))
                }
            }))
            .expect("saver");
        let (tx, rx) = async_channel::unbounded();
        harness
            .run_opencode(
                &OpenCodeTurn {
                    session_id: "chat".into(),
                    run_id: new_opencode_run_id(),
                    prompt: "Build a website for our garden club".into(),
                },
                &tx,
            )
            .await;
        drop(tx);
        let mut events = Vec::new();
        while let Ok(event) = rx.recv().await {
            events.push(event);
        }
        let drafts = drafts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        (events, drafts)
    }

    use sovra_core::Event;
    use sovra_core::artifacts::ArtifactDraft;

    #[tokio::test]
    async fn coding_shortcut_writes_a_page_and_saves_it() {
        let (events, drafts) = run(vec![page(
            "index.html",
            "<title>Garden club</title><p>Meetings on Tuesday.</p>",
        )])
        .await;
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, sovra_core::artifacts::ArtifactKind::Webpage);
        assert_eq!(drafts[0].title, "Garden club");
        assert!(drafts[0].content.contains("Tuesday"));
        let text = events.iter().any(|event| {
            matches!(
                event,
                Event::AgUi { event: sovra_core::AgUiEvent::TextMessageContent { delta, .. }, .. }
                    if delta.contains("garden club page")
            )
        });
        assert!(text, "{events:?}");
        assert!(events.iter().any(|event| {
            matches!(
                event,
                Event::AgUiSurface { widget: sovra_core::AgUiWidget::ArtifactSaved { artifact_kind, .. }, .. }
                    if *artifact_kind == sovra_core::artifacts::ArtifactKind::Webpage
            )
        }));
    }

    #[tokio::test]
    async fn coding_shortcut_saves_a_framework_project() {
        let (_, drafts) = run(vec![
            page("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            page(
                "src/App.tsx",
                "export const Page = () => <h1>Garden club</h1>",
            ),
        ])
        .await;
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].kind, sovra_core::artifacts::ArtifactKind::Project);
        sovra_core::artifacts::WebsiteProject::parse(&drafts[0].content).expect("project");
    }

    #[tokio::test]
    async fn coding_shortcut_says_when_nothing_was_written() {
        let (events, drafts) = run(vec![]).await;
        assert_eq!(drafts, []);
        assert!(events.iter().any(|event| {
            matches!(
                event,
                Event::AgUi { event: sovra_core::AgUiEvent::RunError { message }, .. }
                    if message.contains("didn't write any files")
                        && !message.contains('/')
                        && !message.to_ascii_lowercase().contains("slash")
            )
        }));
    }
    #[derive(Debug)]
    struct TestPiDelivery(std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>);
    #[async_trait::async_trait]
    impl sovra_core::raspberry_pi::PiDeployment for TestPiDelivery {
        fn board_id(&self) -> &'static str {
            "pi-5"
        }
        async fn check(&self) -> Result<(), String> {
            Ok(())
        }
        async fn deploy(
            &self,
            project: sovra_core::raspberry_pi::PiProject,
        ) -> Result<String, String> {
            assert!(project.files.contains_key("main.py"));
            let mut order = self.0.lock().unwrap();
            assert_eq!(*order, vec!["saved"]);
            order.push("deployed");
            Ok("Fixture Pi confirmed the project.".into())
        }
    }

    #[tokio::test]
    async fn raspberry_pi_coding_saves_before_delivery_and_never_delivers_after_a_failed_save() {
        for fail_save in [false, true] {
            let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let saved = order.clone();
            let base = AdkHarness::with_llm_and_tools(
                &AgentConfig::default(),
                std::sync::Arc::new(ScriptLlm {
                    calls: AtomicUsize::new(0),
                    pages: vec![page("main.py", "print('Hello Pi')")],
                }),
                vec![],
            )
            .unwrap();
            let harness = base
                .with_artifact_saver(ArtifactSaver::from_fn(move |draft| {
                    let saved = saved.clone();
                    async move {
                        if fail_save {
                            return Err("Fixture save failure".into());
                        }
                        saved.lock().unwrap().push("saved");
                        Ok(Artifact::from_draft(
                            "pi-source".into(),
                            draft,
                            Timestamp(1),
                        ))
                    }
                }))
                .unwrap();
            let (tx, rx) = async_channel::unbounded();
            harness
                .run_opencode_for_pi(
                    &OpenCodeTurn {
                        session_id: "pi-chat".into(),
                        run_id: new_opencode_run_id(),
                        prompt: "Build a hello program".into(),
                    },
                    &tx,
                    Some(std::sync::Arc::new(TestPiDelivery(order.clone()))),
                )
                .await;
            let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
            if fail_save {
                assert!(order.lock().unwrap().is_empty());
            } else {
                assert_eq!(*order.lock().unwrap(), ["saved", "deployed"]);
            }
            assert_eq!(events.iter().any(|event| matches!(event, Event::AgUi { event: sovra_core::AgUiEvent::TextMessageContent { delta, .. }, .. } if delta.contains("Fixture Pi confirmed"))), !fail_save);
        }
    }
}
