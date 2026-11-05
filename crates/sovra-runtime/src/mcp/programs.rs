//! Programs on this computer that speak MCP over their input and output.
//!
//! Each program gets its own task, so a slow or stuck program never holds up
//! another service. The program runs in its own process group (Unix) and is
//! stopped with everything it started when it is paused, removed, or Sovra
//! quits. The last lines it printed to its error output are kept so the screen
//! can say what went wrong in its own words.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::process::Stdio;
use std::time::Duration;

use async_channel::Sender;
use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use sovra_core::AgUiWidget;
use sovra_core::mcp::{
    McpToolInfo, TOOL_FAILED_TEXT, parse_tool_list, parse_tool_text, parse_tool_widget,
};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::{Mutex, oneshot};

/// First start may download the program (`npx -y`), which can take a while.
const START_TIMEOUT: Duration = Duration::from_secs(120);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How often an idle connection is checked for a program that quit by itself.
const HEALTH_EVERY: Duration = Duration::from_secs(3);
/// Error-output lines kept for the "what went wrong" message.
const STDERR_LINES: usize = 12;
const STDERR_LINE_BYTES: usize = 300;

/// What to start. `private_env` values come from the keychain.
pub(super) struct ProgramSpec {
    pub id: String,
    pub program: String,
    pub args: Vec<String>,
    pub private_env: Vec<(String, String)>,
    pub working_dir: Option<String>,
    /// The login shell's environment, so `npx`/`uvx` installed through shell
    /// profiles are found. `None` inherits the app's own environment.
    pub environment: Option<crate::terminal::LoginEnvironment>,
}

/// News from a program task to the MCP actor.
#[derive(Debug)]
pub(super) enum ProgramUpdate {
    Connected {
        id: String,
        server_name: Option<String>,
        tools: Vec<McpToolInfo>,
    },
    /// It could not start, or it stopped by itself. `detail` is its last words.
    Stopped { id: String, detail: String },
}

pub(super) struct ToolAnswer {
    pub text: String,
    pub widget: Option<AgUiWidget>,
}

impl ToolAnswer {
    fn failed() -> Self {
        Self {
            text: TOOL_FAILED_TEXT.to_owned(),
            widget: None,
        }
    }
}

enum Request {
    Call {
        tool: String,
        arguments: Value,
        reply: oneshot::Sender<ToolAnswer>,
    },
}

/// A running program. Dropping it stops the program and its children.
pub(super) struct ProgramHandle {
    requests: Sender<Request>,
    task: tokio::task::JoinHandle<()>,
}

impl ProgramHandle {
    /// Calls one tool; the answer is the tool's text or a plain failure sentence.
    pub(super) fn call(&self, tool: String, arguments: Value, reply: oneshot::Sender<ToolAnswer>) {
        if let Err(error) = self.requests.try_send(Request::Call {
            tool,
            arguments,
            reply,
        }) {
            let Request::Call { reply, .. } = error.into_inner();
            let _ = reply.send(ToolAnswer::failed());
        }
    }
}

impl Drop for ProgramHandle {
    fn drop(&mut self) {
        // Aborting drops the task's guards, which stop the process group.
        self.task.abort();
    }
}

pub(super) fn start(
    spec: ProgramSpec,
    updates: Sender<super::Input>,
    handle: &tokio::runtime::Handle,
) -> ProgramHandle {
    let (requests, inbox) = async_channel::bounded(64);
    let task = handle.spawn(run(spec, inbox, updates));
    ProgramHandle { requests, task }
}

fn client_info() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("Sovra", env!("CARGO_PKG_VERSION")),
    )
}

/// Finds `name` on `PATH`; on Windows also tries `.exe`, `.cmd` and `.bat`
/// (`npx` is `npx.cmd` there and is not found without its extension).
fn resolve_program(name: &str, search_path: Option<&OsString>) -> OsString {
    if name.contains(['/', '\\']) {
        return OsString::from(name);
    }
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat", ".com"]
    } else {
        &[""]
    };
    search_path
        .into_iter()
        .flat_map(std::env::split_paths)
        .flat_map(|dir| {
            extensions
                .iter()
                .map(move |extension| dir.join(format!("{name}{extension}")))
        })
        .find(|candidate| candidate.is_file())
        .map_or_else(|| OsString::from(name), std::path::PathBuf::into_os_string)
}

async fn run(
    spec: ProgramSpec,
    inbox: async_channel::Receiver<Request>,
    updates: Sender<super::Input>,
) {
    let id = spec.id.clone();
    let (base_env, search_path) = match &spec.environment {
        Some(environment) => (
            environment.variables().await,
            environment.search_path().await,
        ),
        None => (Vec::new(), None),
    };
    let program = {
        let name = spec.program.clone();
        let path = search_path.clone();
        tokio::task::spawn_blocking(move || resolve_program(&name, path.as_ref()))
            .await
            .unwrap_or_else(|_| OsString::from(&spec.program))
    };
    let mut command = tokio::process::Command::new(&program);
    command
        .args(&spec.args)
        .envs(base_env.iter().map(|(key, value)| (key, value)))
        .envs(spec.private_env.iter().map(|(key, value)| (key, value)))
        .kill_on_drop(true);
    if let Some(dir) = &spec.working_dir {
        command.current_dir(dir);
    }
    crate::terminal::own_process_group(&mut command);
    let spawned = TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn();
    let (transport, stderr) = match spawned {
        Ok(parts) => parts,
        Err(error) => {
            tracing::warn!(service_id = %id, %error, "program did not start");
            let detail = if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Couldn't find “{}” on this computer. Install it, or check the spelling.",
                    spec.program
                )
            } else {
                format!("The program couldn't start: {error}")
            };
            let _ = updates
                .send(super::Input::Program(ProgramUpdate::Stopped { id, detail }))
                .await;
            return;
        }
    };
    let _tree = crate::terminal::GroupGuard::new(transport.id());
    let last_words = std::sync::Arc::new(Mutex::new(VecDeque::new()));
    if let Some(stderr) = stderr {
        let sink = std::sync::Arc::clone(&last_words);
        // Ends when the program closes its error output.
        #[expect(
            clippy::disallowed_methods,
            reason = "child of this owned program task; ends with the pipe"
        )]
        tokio::spawn(collect_stderr(stderr, sink));
    }

    let client = match tokio::time::timeout(START_TIMEOUT, client_info().serve(transport)).await {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => {
            tracing::warn!(service_id = %id, %error, "program did not answer its first hello");
            return stopped(
                &updates,
                id,
                &last_words,
                "The program started but didn't answer.",
            )
            .await;
        }
        Err(_) => {
            return stopped(
                &updates,
                id,
                &last_words,
                "The program took too long to start.",
            )
            .await;
        }
    };
    let server_name = client
        .peer_info()
        .and_then(|info| info.server_info.as_ref().map(|server| server.name.clone()))
        .filter(|name| !name.trim().is_empty());
    let tools = match tokio::time::timeout(CALL_TIMEOUT, client.list_all_tools()).await {
        Ok(Ok(tools)) => serde_json::to_value(&tools)
            .map(|tools| parse_tool_list(&json!({ "tools": tools })))
            .unwrap_or_default(),
        Ok(Err(error)) => {
            tracing::warn!(service_id = %id, %error, "program did not list its tools");
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    tracing::info!(service_id = %id, tools = tools.len(), "program connected");
    let _ = updates
        .send(super::Input::Program(ProgramUpdate::Connected {
            id: id.clone(),
            server_name,
            tools,
        }))
        .await;

    let mut health = tokio::time::interval(HEALTH_EVERY);
    health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            request = inbox.recv() => {
                let Ok(Request::Call { tool, arguments, reply }) = request else {
                    break;
                };
                let mut params = CallToolRequestParams::new(tool);
                params.arguments = match arguments {
                    Value::Object(map) => Some(map),
                    _ => None,
                };
                let answer = match tokio::time::timeout(CALL_TIMEOUT, client.call_tool(params)).await {
                    Ok(Ok(result)) => serde_json::to_value(&result)
                        .ok()
                        .and_then(|value| {
                            let text = parse_tool_text(&value);
                            let widget = parse_tool_widget(&value);
                            (!text.is_empty()).then_some(ToolAnswer { text, widget })
                        })
                        .unwrap_or_else(ToolAnswer::failed),
                    Ok(Err(error)) => {
                        tracing::warn!(service_id = %id, %error, "program tool call failed");
                        ToolAnswer::failed()
                    }
                    Err(_) => ToolAnswer {
                        text: "The service took too long to answer.".to_owned(),
                        widget: None,
                    },
                };
                let _ = reply.send(answer);
            }
            _ = health.tick() => {
                if client.is_transport_closed() || client.is_closed() {
                    return stopped(&updates, id, &last_words, "The program stopped by itself.").await;
                }
            }
        }
    }
    // Paused or removed: close politely; the group guard ends any leftovers.
    let _ = tokio::time::timeout(Duration::from_secs(3), client.cancel()).await;
}

async fn stopped(
    updates: &Sender<super::Input>,
    id: String,
    last_words: &Mutex<VecDeque<String>>,
    fallback: &str,
) {
    // Give the error-output reader a moment to catch the final lines.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let lines = last_words.lock().await;
    let detail = if lines.is_empty() {
        fallback.to_owned()
    } else {
        format!(
            "{fallback} It said: {}",
            lines.iter().cloned().collect::<Vec<_>>().join("\n")
        )
    };
    let _ = updates
        .send(super::Input::Program(ProgramUpdate::Stopped { id, detail }))
        .await;
}

async fn collect_stderr(
    stderr: tokio::process::ChildStderr,
    sink: std::sync::Arc<Mutex<VecDeque<String>>>,
) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut kept: String = line.chars().take(STDERR_LINE_BYTES).collect();
        if kept.len() < line.len() {
            kept.push('…');
        }
        let mut buffer = sink.lock().await;
        if buffer.len() == STDERR_LINES {
            buffer.pop_front();
        }
        buffer.push_back(kept);
    }
}
