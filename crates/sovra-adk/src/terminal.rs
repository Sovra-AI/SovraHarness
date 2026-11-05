//! The `run_terminal_command` tool. The runtime behind [`TerminalAccess`] checks
//! the command against the person's rules, shows an approval card when needed,
//! runs it and streams its output to the chat; this file only translates the
//! model's call.

use std::sync::Arc;

use adk_core::{Tool, ToolContext};
use serde_json::{Value, json};
use sovra_core::terminal::{
    DEFAULT_TIMEOUT_SECS, MAX_TIMEOUT_SECS, TERMINAL_TOOL_NAME, TerminalAccess, TerminalRequest,
};

/// The longest a terminal call can take end to end: an unanswered approval card plus
/// the longest command, with a margin for start-up and reporting. ADK cancels tools
/// after five minutes by default.
pub(crate) const TERMINAL_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(
    sovra_core::terminal::APPROVAL_WAIT_SECS + MAX_TIMEOUT_SECS + 60,
);

/// Whether `name` is the terminal tool (the orchestrator lengthens its timeout).
pub(crate) fn is_terminal_tool(name: &str) -> bool {
    name == TERMINAL_TOOL_NAME
}

impl crate::AdkHarness {
    /// Offers the assistant the terminal. Every command goes through the person's
    /// approval rules in the runtime; see `sovra_core::terminal`.
    pub fn with_terminal(&self, access: Arc<dyn TerminalAccess>) -> Result<Self, crate::Error> {
        let mut tools = self.base_tools.clone();
        tools.retain(|tool| !is_terminal_tool(tool.name()));
        if self.tools_enabled {
            tools.push(Arc::new(TerminalTool::new(access)));
        }
        self.rebuild(self.base_instruction.clone(), tools)
    }
}

struct TerminalTool {
    access: Arc<dyn TerminalAccess>,
    description: String,
}

impl TerminalTool {
    fn new(access: Arc<dyn TerminalAccess>) -> Self {
        let description = format!(
            "Run a command in the terminal on the person's computer and get its output. \
             {host} \
             Each command runs in a fresh shell with no keyboard input, so never use \
             interactive programs, editors, pagers, `sudo` or anything that asks for a \
             password; pass flags such as `--yes` instead. Never ask in the chat whether \
             you may run a command or change files, and never paste a command in chat for \
             the person to copy or run themselves: call this tool so they see Allow once, \
             Always allow, and Don't allow on a card. Permission is that card, not a chat \
             question. Always allow remembers that program (for example mkdir) so similar \
             commands can run later without asking. The person sees every command and must \
             approve anything that changes files, installs software or uses the internet, \
             so prefer one clear command over many and explain it in `description`. \
             Read-only commands (ls, cat, grep, git status) usually run straight away. If \
             a command is declined, do not retry it or a variant: ask what they would like \
             instead. Long output is shortened to its start and end.",
            host = access.host().describe_for_model()
        );
        Self {
            access,
            description,
        }
    }
}

#[async_trait::async_trait]
impl Tool for TerminalTool {
    fn name(&self) -> &str {
        TERMINAL_TOOL_NAME
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The exact command line to run."
                },
                "description": {
                    "type": "string",
                    "description": "One short plain-English sentence saying what the command does and why, shown to the person above the command (e.g. \"Run the tests\")."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECS,
                    "description": format!("Stop the command after this many seconds (default {DEFAULT_TIMEOUT_SECS}).")
                },
                "working_directory": {
                    "type": "string",
                    "description": "Absolute folder to run in, when not the default."
                }
            },
            "required": ["command", "description"]
        }))
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn is_concurrency_safe(&self) -> bool {
        // One approval card at a time keeps the conversation readable.
        false
    }

    async fn execute(&self, ctx: Arc<dyn ToolContext>, args: Value) -> adk_core::Result<Value> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };
        let Some(command) = text("command") else {
            return Err(adk_core::AdkError::tool("command is required"));
        };
        let request = TerminalRequest {
            session_id: ctx.session_id().to_owned(),
            call_id: ctx.function_call_id().to_owned(),
            command: command.to_owned(),
            description: text("description").unwrap_or_default().to_owned(),
            timeout_secs: args
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_TIMEOUT_SECS),
            working_dir: text("working_directory").map(str::to_owned),
        };
        let outcome = self.access.run(request).await;
        Ok(outcome.report_for_model())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sovra_core::terminal::{CallStatus, HostOs, HostSystem, ShellKind, TerminalOutcome};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct Recorder {
        host: HostSystem,
        seen: Mutex<Vec<TerminalRequest>>,
    }

    #[async_trait::async_trait]
    impl TerminalAccess for Recorder {
        fn host(&self) -> &HostSystem {
            &self.host
        }

        async fn run(&self, request: TerminalRequest) -> TerminalOutcome {
            self.seen.lock().expect("test lock").push(request);
            TerminalOutcome::Ran {
                status: CallStatus::Finished {
                    exit_code: Some(0),
                    duration_ms: 3,
                },
                output: "hello".into(),
                total_lines: 1,
            }
        }
    }

    struct FakeToolContext {
        session_id: String,
        call_id: String,
        content: adk_core::Content,
        actions: Mutex<adk_core::EventActions>,
    }

    impl FakeToolContext {
        fn new(session_id: &str, call_id: &str) -> Self {
            Self {
                session_id: session_id.into(),
                call_id: call_id.into(),
                content: adk_core::Content::new("user"),
                actions: Mutex::new(adk_core::EventActions::default()),
            }
        }
    }

    #[async_trait::async_trait]
    impl adk_core::ReadonlyContext for FakeToolContext {
        fn invocation_id(&self) -> &'static str {
            "invocation"
        }
        fn agent_name(&self) -> &'static str {
            "sovra"
        }
        fn user_id(&self) -> &'static str {
            "user"
        }
        fn app_name(&self) -> &'static str {
            "sovra"
        }
        fn session_id(&self) -> &str {
            &self.session_id
        }
        fn branch(&self) -> &'static str {
            ""
        }
        fn user_content(&self) -> &adk_core::Content {
            &self.content
        }
    }

    #[async_trait::async_trait]
    impl adk_core::CallbackContext for FakeToolContext {
        fn artifacts(&self) -> Option<Arc<dyn adk_core::Artifacts>> {
            None
        }
    }

    #[async_trait::async_trait]
    impl ToolContext for FakeToolContext {
        fn function_call_id(&self) -> &str {
            &self.call_id
        }
        fn actions(&self) -> adk_core::EventActions {
            self.actions.lock().expect("test lock").clone()
        }
        fn set_actions(&self, actions: adk_core::EventActions) {
            *self.actions.lock().expect("test lock") = actions;
        }
        async fn search_memory(
            &self,
            _query: &str,
        ) -> adk_core::Result<Vec<adk_core::MemoryEntry>> {
            Ok(Vec::new())
        }
    }

    fn recorder() -> Arc<Recorder> {
        Arc::new(Recorder {
            host: HostSystem {
                os: HostOs::Linux,
                os_version: "Ubuntu 24.04".into(),
                arch: "x86_64".into(),
                shell: ShellKind::Bash,
                shell_path: "/usr/bin/bash".into(),
                working_dir: "/home/ada".into(),
            },
            seen: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn description_tells_the_model_about_this_computer() {
        let tool = TerminalTool::new(recorder());
        assert!(tool.description().contains("Ubuntu 24.04"));
        assert!(tool.description().contains("bash"));
        assert!(tool.description().contains("Always allow"));
        assert!(tool.description().contains("mkdir"));
        assert!(tool.description().contains("Never ask in the chat"));
        assert!(
            tool.description()
                .contains("paste a command in chat for the person to copy or run"),
            "{}",
            tool.description()
        );
        assert!(
            tool.description().contains("call this tool"),
            "{}",
            tool.description()
        );
        let schema = tool.parameters_schema().expect("schema");
        assert_eq!(schema["required"], json!(["command", "description"]));
        assert!(is_terminal_tool(tool.name()));
    }

    #[test]
    fn tool_timeout_covers_approval_and_longest_command() {
        assert!(
            TERMINAL_TOOL_TIMEOUT.as_secs()
                > sovra_core::terminal::APPROVAL_WAIT_SECS + MAX_TIMEOUT_SECS
        );
    }

    #[tokio::test]
    async fn execute_forwards_the_request_and_reports() {
        let access = recorder();
        let tool = TerminalTool::new(access.clone());
        let ctx: Arc<dyn ToolContext> = Arc::new(FakeToolContext::new("s1", "call-9"));
        let report = tool
            .execute(
                ctx,
                json!({"command": " ls -la ", "description": "List files", "timeout_seconds": 5}),
            )
            .await
            .expect("tool ran");
        assert_eq!(report["status"], "finished");
        assert_eq!(report["output"], "hello");
        let seen = access.seen.lock().expect("test lock");
        assert_eq!(seen[0].command, "ls -la");
        assert_eq!(seen[0].call_id, "call-9");
        assert_eq!(seen[0].session_id, "s1");
        assert_eq!(seen[0].timeout_secs, 5);
    }

    #[tokio::test]
    async fn execute_rejects_a_missing_command() {
        let tool = TerminalTool::new(recorder());
        let ctx: Arc<dyn ToolContext> = Arc::new(FakeToolContext::new("s1", "c"));
        assert!(
            tool.execute(ctx, json!({"description": "x"}))
                .await
                .is_err()
        );
    }
}
