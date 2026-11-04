//! Terminal access: the rules deciding whether a command the assistant wants to run
//! may run without asking, the approval protocol, and the state the chat renders.
//!
//! Every command is checked here before the runtime starts a process:
//!
//! 1. [`parse::analyse`] splits it into simple commands (`a && b | c`).
//! 2. Each simple command must be read-only ([`classify::is_read_only`]) or match one of
//!    the person's saved [`AllowRule`]s; otherwise the person is asked.
//! 3. The person answers [`TerminalDecision::Deny`], [`TerminalDecision::AllowOnce`] or
//!    [`TerminalDecision::AlwaysAllow`] (which saves the suggested rules).
//!
//! Commands Sovra cannot fully read, and commands that can run arbitrary code or are
//! destructive, always ask and are never remembered. Text rules are a convenience,
//! not a sandbox: an approved command runs with the person's own rights. Design and
//! sources: `docs/terminal.md`.

mod classify;
mod output;
mod parse;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::fmt;

use serde::{Deserialize, Serialize};

pub use classify::{Risk, always_asks, is_read_only, suggested_rule};
pub use output::{LineDecoder, MAX_LINE_BYTES, MODEL_HEAD_BYTES, MODEL_TAIL_BYTES, OutputCapture};
pub use parse::{
    CommandAnalysis, Joiner, MAX_ANALYSED_COMMAND_BYTES, Opaque, Redirect, SimpleCommand, Word,
    analyse, program_name,
};

/// The tool name the assistant calls.
pub const TERMINAL_TOOL_NAME: &str = "run_terminal_command";
/// Used when the assistant does not ask for a timeout.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// The longest a single command may run.
pub const MAX_TIMEOUT_SECS: u64 = 600;
/// How long an approval card waits for an answer before the command is dropped.
pub const APPROVAL_WAIT_SECS: u64 = 600;
/// Most "always allow" rules one answer may save.
pub const MAX_RULES_PER_APPROVAL: usize = 5;
/// Output lines the chat keeps per command (the model gets its own bounded copy).
pub const UI_MAX_LINES: usize = 500;
/// Commands the chat remembers; older finished ones are forgotten first.
pub const UI_MAX_CALLS: usize = 200;

/// Environment set on every command so tools print plain text and never wait for
/// input that cannot arrive (stdin is closed).
pub const NON_INTERACTIVE_ENV: &[(&str, &str)] = &[
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    ("TERM", "dumb"),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GCM_INTERACTIVE", "never"),
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("PIP_NO_INPUT", "1"),
    ("POWERSHELL_TELEMETRY_OPTOUT", "1"),
    ("SOVRA_TERMINAL", "1"),
];

/// The operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HostOs {
    Linux,
    MacOs,
    Windows,
    /// A BSD or another Unix.
    OtherUnix,
}

impl HostOs {
    /// The family this binary was compiled for.
    pub fn current() -> Self {
        match std::env::consts::OS {
            "linux" => Self::Linux,
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            _ => Self::OtherUnix,
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::OtherUnix => "Unix",
        }
    }
}

/// The shell commands run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShellKind {
    Bash,
    Zsh,
    /// POSIX `sh`.
    Sh,
    /// `pwsh` (PowerShell 7) or Windows `PowerShell` 5.1.
    PowerShell,
}

impl ShellKind {
    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Sh => "sh",
            Self::PowerShell => "PowerShell",
        }
    }

    /// Arguments that make the shell run `command` once, non-interactively, without
    /// reading the person's interactive start-up files.
    pub fn invocation(self, command: &str) -> Vec<String> {
        match self {
            Self::Bash | Self::Zsh | Self::Sh => vec!["-c".to_owned(), command.to_owned()],
            Self::PowerShell => vec![
                "-NoLogo".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                // UTF-8 output and no progress bars, then the command itself.
                format!(
                    "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
                     $ProgressPreference='SilentlyContinue'; {command}"
                ),
            ],
        }
    }
}

/// Picks the shell: on Windows `pwsh`, else Windows `PowerShell`; elsewhere the
/// person's login shell when it is bash or zsh (so their syntax habits work), else
/// bash, else `/bin/sh`. `find` looks a program up on `PATH`.
pub fn choose_shell(
    os: HostOs,
    login_shell: Option<&str>,
    find: impl Fn(&str) -> Option<String>,
) -> (ShellKind, String) {
    if os == HostOs::Windows {
        let path = find("pwsh")
            .or_else(|| find("powershell"))
            .unwrap_or_else(|| "powershell.exe".to_owned());
        return (ShellKind::PowerShell, path);
    }
    if let Some(login) = login_shell.filter(|path| !path.is_empty()) {
        match program_name(login).as_str() {
            "bash" => return (ShellKind::Bash, login.to_owned()),
            "zsh" => return (ShellKind::Zsh, login.to_owned()),
            _ => {}
        }
    }
    match find("bash") {
        Some(path) => (ShellKind::Bash, path),
        None => (ShellKind::Sh, "/bin/sh".to_owned()),
    }
}

/// `PRETTY_NAME` from Linux `/etc/os-release` text (e.g. `Ubuntu 24.04.1 LTS`).
pub fn os_release_name(text: &str) -> Option<String> {
    let mut name = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']).trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "PRETTY_NAME" => return Some(value.to_owned()),
            "NAME" => name = Some(value.to_owned()),
            _ => {}
        }
    }
    name
}

/// The version from Windows `ver` output (`Microsoft Windows [Version 10.0.26100.2314]`),
/// named the way people know it: build 22000 and later is Windows 11.
pub fn windows_version_name(ver_output: &str) -> Option<String> {
    let start = ver_output.find("Version ")? + "Version ".len();
    let version: String = ver_output[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let build: u32 = version.split('.').nth(2)?.parse().ok()?;
    let family = if build >= 22_000 {
        "Windows 11"
    } else {
        "Windows 10"
    };
    Some(format!("{family} (build {build}, {version})"))
}

/// What the assistant needs to know to write commands that work here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSystem {
    pub os: HostOs,
    /// e.g. `Ubuntu 24.04`, `macOS 15.1`, `Windows 11 (26100)`.
    pub os_version: String,
    /// `x86_64`, `aarch64`, …
    pub arch: String,
    pub shell: ShellKind,
    pub shell_path: String,
    /// Commands start here unless the assistant asks for another folder.
    pub working_dir: String,
}

impl HostSystem {
    /// The paragraph added to the tool description.
    pub fn describe_for_model(&self) -> String {
        let syntax = match self.shell {
            ShellKind::PowerShell => {
                "Write PowerShell syntax (Get-ChildItem, $env:NAME, `;` between statements). \
                 Unix tools such as grep or sed may be missing."
            }
            ShellKind::Bash | ShellKind::Zsh | ShellKind::Sh => {
                "Write POSIX shell syntax. Use the platform's own tools (on macOS, BSD flags \
                 such as `sed -i ''`)."
            }
        };
        format!(
            "This computer runs {os} ({version}, {arch}). Commands run in {shell} at {shell_path}, \
             starting in {dir}. {syntax}",
            os = self.os.label(),
            version = self.os_version,
            arch = self.arch,
            shell = self.shell.label(),
            shell_path = self.shell_path,
            dir = self.working_dir,
        )
    }
}

/// A saved "always allow" rule: the command's first words. `npm test` allows
/// `npm test`, `npm test --watch` and so on, but not `npm install`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AllowRule {
    pub words: Vec<String>,
}

impl AllowRule {
    /// A rule from words; `None` when empty.
    pub fn new(words: Vec<String>) -> Option<Self> {
        (!words.is_empty() && words.iter().all(|word| !word.trim().is_empty()))
            .then_some(Self { words })
    }

    /// Whether this rule approves `command`. Commands that always ask, set unusual
    /// environment variables, or hide their words behind variables never match.
    pub fn matches(&self, command: &SimpleCommand, os: HostOs) -> bool {
        if always_asks(command) || !parse::assignments_are_safe(command) {
            return false;
        }
        if command.words.len() < self.words.len() {
            return false;
        }
        self.words
            .iter()
            .zip(&command.words)
            .enumerate()
            .all(|(index, (rule, word))| {
                if word.dynamic || word.glob {
                    return false;
                }
                if index == 0 && os == HostOs::Windows {
                    // Windows finds `Git.EXE` for `git`.
                    return program_name(rule) == program_name(&word.text);
                }
                *rule == word.text
            })
    }
}

impl fmt::Display for AllowRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.words.join(" "))
    }
}

/// The person's terminal settings, saved between runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalPolicy {
    /// Whether the assistant is offered the terminal at all.
    pub enabled: bool,
    /// "Always allow" rules, oldest first.
    pub rules: Vec<AllowRule>,
}

impl Default for TerminalPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            rules: Vec::new(),
        }
    }
}

/// Why a command may run without asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoApproval {
    /// Every part only reads.
    ReadOnly,
    /// Saved rules (and read-only parts) cover every part.
    Rules(Vec<AllowRule>),
}

/// What happens to a command before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Run(AutoApproval),
    Ask(ApprovalPrompt),
}

/// What the approval card shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApprovalPrompt {
    /// Plain-English warnings, most serious first.
    pub risks: Vec<Risk>,
    /// Rules "Always allow" would save; empty when the command must be asked every time.
    pub suggested_rules: Vec<AllowRule>,
}

impl TerminalPolicy {
    /// Decides whether `command` runs now or asks first.
    pub fn evaluate(&self, command: &str, os: HostOs) -> Verdict {
        let analysis = analyse(command);
        let mut risks = Vec::new();
        for simple in &analysis.commands {
            classify::risks(simple, &mut risks);
        }
        if classify::pipes_download_into_code(&analysis.commands) {
            risks.push(Risk::RunsDownloadedCode);
        }
        if analysis.has_background() {
            risks.push(Risk::RunsInBackground);
        }
        if !analysis.is_clear() || analysis.commands.is_empty() {
            risks.push(Risk::HardToCheck);
            return Verdict::Ask(ApprovalPrompt {
                risks: sorted(risks),
                suggested_rules: Vec::new(),
            });
        }

        let mut used = Vec::new();
        let mut unapproved = Vec::new();
        for simple in &analysis.commands {
            if is_read_only(simple) {
                continue;
            }
            match self.rules.iter().find(|rule| rule.matches(simple, os)) {
                Some(rule) => {
                    if !used.contains(rule) {
                        used.push(rule.clone());
                    }
                }
                None => unapproved.push(simple),
            }
        }
        // Background jobs outlive the timeout and Stop button, so always ask.
        if unapproved.is_empty() && !analysis.has_background() {
            return Verdict::Run(if used.is_empty() {
                AutoApproval::ReadOnly
            } else {
                AutoApproval::Rules(used)
            });
        }

        let mut suggested_rules = Vec::new();
        let mut rememberable = !analysis.has_background();
        for simple in unapproved {
            match suggested_rule(simple).and_then(AllowRule::new) {
                Some(rule) if parse::assignments_are_safe(simple) => {
                    if !suggested_rules.contains(&rule) {
                        suggested_rules.push(rule);
                    }
                }
                _ => rememberable = false,
            }
        }
        if !rememberable || suggested_rules.len() > MAX_RULES_PER_APPROVAL {
            suggested_rules.clear();
        }
        Verdict::Ask(ApprovalPrompt {
            risks: sorted(risks),
            suggested_rules,
        })
    }

    /// Adds rules, skipping duplicates. Returns whether anything changed.
    pub fn add_rules(&mut self, rules: &[AllowRule]) -> bool {
        let before = self.rules.len();
        for rule in rules {
            if !self.rules.contains(rule) {
                self.rules.push(rule.clone());
            }
        }
        self.rules.len() != before
    }

    /// Removes a rule. Returns whether it was present.
    pub fn remove_rule(&mut self, rule: &AllowRule) -> bool {
        let before = self.rules.len();
        self.rules.retain(|existing| existing != rule);
        self.rules.len() != before
    }
}

fn sorted(mut risks: Vec<Risk>) -> Vec<Risk> {
    risks.sort_by_key(|risk| (!risk.is_serious(), *risk));
    risks.dedup();
    risks
}

/// The person's answer to an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TerminalDecision {
    /// Don't run it this time.
    Deny,
    /// Run it this time only.
    AllowOnce,
    /// Run it and save the suggested rules so similar commands run without asking.
    AlwaysAllow,
}

/// What the assistant asked to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRequest {
    pub session_id: String,
    /// The model's tool-call id, unique within the run.
    pub call_id: String,
    pub command: String,
    /// The assistant's one-line explanation, shown above the command.
    pub description: String,
    pub timeout_secs: u64,
    /// A folder to start in instead of the default.
    pub working_dir: Option<String>,
}

impl TerminalRequest {
    /// Clamps the timeout into `1..=MAX_TIMEOUT_SECS`.
    pub fn bounded_timeout_secs(&self) -> u64 {
        self.timeout_secs.clamp(1, MAX_TIMEOUT_SECS)
    }
}

/// Where a command is in its life.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallStatus {
    /// Waiting for the person to answer the approval card.
    AwaitingApproval,
    Running,
    /// Exited on its own. `exit_code` is `None` when a signal ended it.
    Finished {
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    /// Stopped because it ran past its time limit.
    TimedOut {
        duration_ms: u64,
    },
    /// Stopped by the person or because the conversation was interrupted.
    Stopped {
        duration_ms: u64,
    },
    /// The person chose "Don't allow".
    Declined,
    /// Nobody answered in time, or the conversation moved on.
    Expired,
    /// The person turned terminal access off.
    Disabled,
    /// The shell could not be started.
    FailedToStart {
        message: String,
    },
}

impl CallStatus {
    /// Whether the command is finished one way or another.
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::AwaitingApproval | Self::Running)
    }

    /// Whether the command ran and ended badly, or never could start.
    pub fn is_failure(&self) -> bool {
        match self {
            Self::Finished { exit_code, .. } => *exit_code != Some(0),
            Self::TimedOut { .. } | Self::FailedToStart { .. } => true,
            Self::AwaitingApproval
            | Self::Running
            | Self::Stopped { .. }
            | Self::Declined
            | Self::Expired
            | Self::Disabled => false,
        }
    }

    /// The status line on the card, in plain words.
    pub fn label(&self) -> String {
        match self {
            Self::AwaitingApproval => "Waiting for your OK".to_owned(),
            Self::Running => "Running…".to_owned(),
            Self::Finished {
                exit_code: Some(0),
                duration_ms,
            } => format!("Done in {}", format_duration(*duration_ms)),
            Self::Finished {
                exit_code: Some(code),
                duration_ms,
            } => format!(
                "Finished with error code {code} after {}",
                format_duration(*duration_ms)
            ),
            Self::Finished {
                exit_code: None,
                duration_ms,
            } => format!(
                "Ended by the system after {}",
                format_duration(*duration_ms)
            ),
            Self::TimedOut { duration_ms } => format!(
                "Stopped: still running after {}",
                format_duration(*duration_ms)
            ),
            Self::Stopped { duration_ms } => {
                format!("Stopped after {}", format_duration(*duration_ms))
            }
            Self::Declined => "Not allowed".to_owned(),
            Self::Expired => "Not answered, so it didn't run".to_owned(),
            Self::Disabled => "Terminal access is switched off".to_owned(),
            Self::FailedToStart { message } => message.clone(),
        }
    }
}

/// `850ms`, `4.2s`, `3m 05s`.
pub fn format_duration(duration_ms: u64) -> String {
    match duration_ms {
        0..1_000 => format!("{duration_ms}ms"),
        1_000..60_000 => format!("{}.{}s", duration_ms / 1_000, duration_ms % 1_000 / 100),
        _ => {
            let seconds = duration_ms / 1_000;
            format!("{}m {:02}s", seconds / 60, seconds % 60)
        }
    }
}

/// How a call got permission to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    /// Not decided yet.
    Pending,
    Auto(AutoApproval),
    /// The person approved it once.
    Once,
    /// The person approved it and saved these rules.
    Saved(Vec<AllowRule>),
    Refused,
}

impl Permission {
    /// Why the command was allowed to run, for the card's footer.
    pub fn label(&self) -> Option<String> {
        match self {
            Self::Auto(AutoApproval::ReadOnly) => {
                Some("Ran without asking because it only looks at things".to_owned())
            }
            Self::Auto(AutoApproval::Rules(rules)) => Some(format!(
                "Ran without asking: you always allow {}",
                quoted_rules(rules)
            )),
            Self::Once => Some("You allowed this once".to_owned()),
            Self::Saved(rules) => Some(format!(
                "You allowed this. From now on {} will run without asking",
                quoted_rules(rules)
            )),
            Self::Pending | Self::Refused => None,
        }
    }
}

/// `“npm test”` or `“npm test” and “cargo build”`.
pub fn quoted_rules(rules: &[AllowRule]) -> String {
    let quoted: Vec<String> = rules.iter().map(|rule| format!("“{rule}”")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// One terminal command as the chat shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCall {
    pub call_id: String,
    pub session_id: String,
    pub command: String,
    pub description: String,
    pub working_dir: String,
    pub status: CallStatus,
    pub permission: Permission,
    pub prompt: ApprovalPrompt,
}

/// A request from the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalCommand {
    /// Answer an approval card.
    Decide {
        call_id: String,
        decision: TerminalDecision,
    },
    /// Stop a running command.
    Stop { call_id: String },
    /// Forget a saved "always allow" rule.
    RemoveRule(AllowRule),
    /// Offer or withdraw the terminal from the assistant.
    SetEnabled(bool),
}

/// A notification to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalEvent {
    /// Sent once at start-up.
    Ready {
        host: HostSystem,
        policy: TerminalPolicy,
    },
    PolicyChanged(TerminalPolicy),
    /// A call appeared or changed status.
    Call(TerminalCall),
    /// New output for a running call. `partial` is the unfinished last line;
    /// `skipped_lines` were printed too fast to forward and are only counted.
    Output {
        call_id: String,
        lines: Vec<String>,
        partial: String,
        skipped_lines: u64,
    },
}

/// One call plus the output the chat keeps for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallView {
    pub call: TerminalCall,
    /// The last [`UI_MAX_LINES`] complete lines.
    pub lines: VecDeque<String>,
    /// Lines dropped from the front.
    pub omitted_lines: u64,
    /// The unfinished last line.
    pub partial: String,
}

/// Terminal state folded from [`TerminalEvent`]s for the UI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalState {
    pub host: Option<HostSystem>,
    pub policy: TerminalPolicy,
    calls: VecDeque<CallView>,
}

impl TerminalState {
    /// Folds one event.
    pub fn apply(&mut self, event: &TerminalEvent) {
        match event {
            TerminalEvent::Ready { host, policy } => {
                self.host = Some(host.clone());
                self.policy = policy.clone();
            }
            TerminalEvent::PolicyChanged(policy) => self.policy = policy.clone(),
            TerminalEvent::Call(call) => {
                if let Some(index) = self.position(&call.call_id) {
                    self.calls[index].call = call.clone();
                } else {
                    self.calls.push_back(CallView {
                        call: call.clone(),
                        lines: VecDeque::new(),
                        omitted_lines: 0,
                        partial: String::new(),
                    });
                    self.trim();
                }
            }
            TerminalEvent::Output {
                call_id,
                lines,
                partial,
                skipped_lines,
            } => {
                let Some(index) = self.position(call_id) else {
                    return;
                };
                let view = &mut self.calls[index];
                view.partial.clone_from(partial);
                view.omitted_lines += skipped_lines;
                for line in lines {
                    if view.lines.len() == UI_MAX_LINES {
                        view.lines.pop_front();
                        view.omitted_lines += 1;
                    }
                    view.lines.push_back(line.clone());
                }
            }
        }
    }

    fn position(&self, call_id: &str) -> Option<usize> {
        self.calls
            .iter()
            .rposition(|view| view.call.call_id == call_id)
    }

    fn trim(&mut self) {
        while self.calls.len() > UI_MAX_CALLS {
            match self
                .calls
                .iter()
                .position(|view| view.call.status.is_settled())
            {
                Some(index) => {
                    self.calls.remove(index);
                }
                None => break,
            }
        }
    }

    /// Every remembered call, oldest first, with its index for [`Self::call_at`].
    pub fn calls(&self) -> impl Iterator<Item = (usize, &CallView)> {
        self.calls.iter().enumerate()
    }

    /// The call at an index from [`Self::calls`].
    pub fn call_at(&self, index: usize) -> Option<&CallView> {
        self.calls.get(index)
    }

    /// A call by id.
    pub fn call(&self, call_id: &str) -> Option<&CallView> {
        self.position(call_id).map(|index| &self.calls[index])
    }

    /// Calls for one conversation, oldest first.
    pub fn calls_for<'a>(&'a self, session_id: &'a str) -> impl Iterator<Item = &'a CallView> {
        self.calls
            .iter()
            .filter(move |view| view.call.session_id == session_id)
    }

    /// Whether any call in the conversation is waiting for an answer.
    pub fn awaiting_approval(&self, session_id: &str) -> bool {
        self.calls_for(session_id)
            .any(|view| view.call.status == CallStatus::AwaitingApproval)
    }
}

/// What the tool returns to the assistant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalOutcome {
    /// The command ran (or was stopped while running).
    Ran {
        status: CallStatus,
        output: String,
        total_lines: u64,
    },
    /// It never ran.
    NotRun(CallStatus),
}

impl TerminalOutcome {
    /// The JSON tool result. Wording steers the model: a refusal is final for this
    /// command, and a missing tool is not the person's fault.
    pub fn report_for_model(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Ran {
                status,
                output,
                total_lines,
            } => {
                let (state, exit_code, duration_ms) = match status {
                    CallStatus::Finished {
                        exit_code,
                        duration_ms,
                    } => ("finished", *exit_code, *duration_ms),
                    CallStatus::TimedOut { duration_ms } => ("timed_out", None, *duration_ms),
                    CallStatus::Stopped { duration_ms } => {
                        ("stopped_by_person", None, *duration_ms)
                    }
                    other => return Self::NotRun(other.clone()).report_for_model(),
                };
                json!({
                    "status": state,
                    "exit_code": exit_code,
                    "duration_ms": duration_ms,
                    "output_lines": total_lines,
                    "output": output,
                })
            }
            Self::NotRun(status) => {
                let (state, note) = match status {
                    CallStatus::Declined => (
                        "declined",
                        "The person chose not to allow this command. Do not retry it or a \
                         variant; ask them what they would like instead.",
                    ),
                    CallStatus::Expired => (
                        "not_answered",
                        "Nobody approved the command in time, so it did not run.",
                    ),
                    CallStatus::Disabled => {
                        ("disabled", "Terminal access is switched off in Settings.")
                    }
                    CallStatus::FailedToStart { message } => {
                        return json!({"status": "failed_to_start", "error": message});
                    }
                    CallStatus::AwaitingApproval
                    | CallStatus::Running
                    | CallStatus::Finished { .. }
                    | CallStatus::TimedOut { .. }
                    | CallStatus::Stopped { .. } => ("not_run", "The command did not run."),
                };
                json!({"status": state, "note": note})
            }
        }
    }
}

/// The runtime's terminal, as the assistant's tool sees it. Implemented in
/// `sovra-runtime`, which owns processes and the approval wait.
#[async_trait::async_trait]
pub trait TerminalAccess: Send + Sync + fmt::Debug {
    /// The computer commands run on.
    fn host(&self) -> &HostSystem;
    /// Checks, asks if needed, runs and reports. Dropping the future stops the command.
    async fn run(&self, request: TerminalRequest) -> TerminalOutcome;
}
