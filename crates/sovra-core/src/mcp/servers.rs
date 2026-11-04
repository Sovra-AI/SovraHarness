//! Managing many connected services: programs on this computer as well as web
//! addresses, settings pasted from other apps, the safety review shown before a
//! program first runs, and the search/summary the list screen needs.
//!
//! Pure rules only; `sovra-runtime` starts programs and talks to them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};

use super::{McpPhase, McpService, McpState};

/// Most services one person can keep. A tool list this long is already more than
/// a model can use well; the cap keeps a pasted file from flooding the screen.
pub const MAX_SERVICES: usize = 100;
/// Command-line flag that starts Sovra's built-in sample MCP server.
pub const SAMPLE_SERVER_FLAG: &str = "--sample-mcp-server";
/// Longest tool name chat models accept (`OpenAI` and Gemini both cap at 64).
pub const MAX_TOOL_NAME: usize = 64;

/// How Sovra reaches a service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// A web address (Streamable HTTP).
    #[default]
    Web,
    /// A program on this computer that Sovra starts and talks to through its
    /// input and output. Environment *values* are secrets and live in the
    /// keychain; only their names are kept here.
    Program {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env_names: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
}

impl McpTransport {
    /// Whether this is a program on this computer.
    pub fn is_program(&self) -> bool {
        matches!(self, Self::Program { .. })
    }
}

/// One service read from pasted settings or typed into the add form, before it
/// is saved. `Debug` hides header and environment values.
#[derive(Clone, PartialEq, Eq)]
pub struct ServerDraft {
    pub name: String,
    pub enabled: bool,
    pub setup: DraftSetup,
}

/// Where a drafted service lives and the private settings it needs.
#[derive(Clone, PartialEq, Eq)]
pub enum DraftSetup {
    Web {
        address: String,
        headers: Vec<(String, String)>,
    },
    Program {
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        working_dir: Option<String>,
    },
}

fn names_only(pairs: &[(String, String)]) -> Vec<&str> {
    pairs.iter().map(|(name, _)| name.as_str()).collect()
}

impl std::fmt::Debug for ServerDraft {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerDraft")
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .field("setup", &self.setup)
            .finish()
    }
}

impl std::fmt::Debug for DraftSetup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Web { address, headers } => formatter
                .debug_struct("Web")
                .field("address", address)
                .field("headers", &names_only(headers))
                .finish(),
            Self::Program {
                command,
                args,
                env,
                working_dir,
            } => formatter
                .debug_struct("Program")
                .field("command", command)
                .field("args", args)
                .field("env", &names_only(env))
                .field("working_dir", working_dir)
                .finish(),
        }
    }
}

/// Why pasted settings could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportProblem {
    Empty,
    NotUnderstood,
    NoServices,
    TooMany,
    /// A command that chains several commands or redirects output.
    NotOneProgram,
}

impl ImportProblem {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Empty => {
                "Paste the settings, a web address, or the command the service's instructions give."
            }
            Self::NotUnderstood => {
                "Sovra couldn't read that. Paste the settings exactly as the instructions show them."
            }
            Self::NoServices => "There are no services in what you pasted.",
            Self::TooMany => "That's more services than Sovra can keep. Paste fewer at a time.",
            Self::NotOneProgram => {
                "Paste a single program and its options, without && or other commands."
            }
        }
    }
}

/// Reads what a person pasted: the `mcpServers` settings used by Claude Desktop
/// and Cursor, VS Code's `servers`, Zed's `context_servers`, a single server
/// object, a web address, or a command line such as
/// `npx -y @modelcontextprotocol/server-filesystem ~/Documents`.
pub fn parse_server_settings(text: &str) -> Result<Vec<ServerDraft>, ImportProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ImportProblem::Empty);
    }
    if text.starts_with('{') {
        let value: Value = serde_json::from_str(text).map_err(|_| ImportProblem::NotUnderstood)?;
        return drafts_from_json(&value);
    }
    if text.starts_with("http://") || text.starts_with("https://") {
        return Ok(vec![ServerDraft {
            name: String::new(),
            enabled: true,
            setup: DraftSetup::Web {
                address: text.to_owned(),
                headers: Vec::new(),
            },
        }]);
    }
    program_from_command_line(text).map(|draft| vec![draft])
}

fn drafts_from_json(value: &Value) -> Result<Vec<ServerDraft>, ImportProblem> {
    let root = value.as_object().ok_or(ImportProblem::NotUnderstood)?;
    let map = ["mcpServers", "servers", "context_servers"]
        .iter()
        .find_map(|key| root.get(*key).and_then(Value::as_object))
        .or_else(|| {
            root.get("mcp")
                .and_then(|mcp| mcp.get("servers"))
                .and_then(Value::as_object)
        });
    let entries: Vec<(String, &Map<String, Value>)> = match map {
        Some(map) => map
            .iter()
            .filter_map(|(name, entry)| Some((name.clone(), entry.as_object()?)))
            .collect(),
        // A single server object, or a bare map of name → server.
        None if looks_like_server(root) => vec![(String::new(), root)],
        None => root
            .iter()
            .filter_map(|(name, entry)| {
                let entry = entry.as_object()?;
                looks_like_server(entry).then(|| (name.clone(), entry))
            })
            .collect(),
    };
    if entries.is_empty() {
        return Err(ImportProblem::NoServices);
    }
    if entries.len() > MAX_SERVICES {
        return Err(ImportProblem::TooMany);
    }
    entries
        .into_iter()
        .map(|(name, entry)| draft_from_entry(name, entry))
        .collect()
}

fn looks_like_server(entry: &Map<String, Value>) -> bool {
    ["command", "url", "serverUrl", "httpUrl"]
        .iter()
        .any(|key| entry.contains_key(*key))
}

fn string_pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(name, value)| {
                    let text = match value {
                        Value::String(text) => text.clone(),
                        Value::Number(number) => number.to_string(),
                        Value::Bool(flag) => flag.to_string(),
                        _ => return None,
                    };
                    (!name.trim().is_empty()).then(|| (name.trim().to_owned(), text))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn draft_from_entry(
    name: String,
    entry: &Map<String, Value>,
) -> Result<ServerDraft, ImportProblem> {
    let enabled = !entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
    let address = ["url", "serverUrl", "httpUrl"]
        .iter()
        .find_map(|key| entry.get(*key).and_then(Value::as_str));
    if let Some(address) = address {
        return Ok(ServerDraft {
            name,
            enabled,
            setup: DraftSetup::Web {
                address: address.trim().to_owned(),
                headers: string_pairs(entry.get("headers")),
            },
        });
    }
    // Zed nests the program: `"command": {"path", "args", "env"}`.
    let (command, mut args, mut env) = match entry.get("command") {
        Some(Value::String(command)) => (
            command.trim().to_owned(),
            strings(entry.get("args")),
            string_pairs(entry.get("env")),
        ),
        Some(Value::Object(nested)) => (
            nested
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            strings(nested.get("args")),
            string_pairs(nested.get("env")),
        ),
        _ => return Err(ImportProblem::NotUnderstood),
    };
    if command.is_empty() {
        return Err(ImportProblem::NotUnderstood);
    }
    // `"command": "npx -y pkg"` with no args: split it the way a shell would.
    let command = if args.is_empty() && command.contains(' ') {
        let parsed = program_from_command_line(&command)?;
        let DraftSetup::Program {
            command,
            args: parsed_args,
            env: parsed_env,
            ..
        } = parsed.setup
        else {
            return Err(ImportProblem::NotUnderstood);
        };
        args = parsed_args;
        env.extend(parsed_env);
        command
    } else {
        command
    };
    let working_dir = entry
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(str::to_owned);
    Ok(ServerDraft {
        name,
        enabled,
        setup: DraftSetup::Program {
            command,
            args,
            env,
            working_dir,
        },
    })
}

/// A typed command line becomes a program draft. `NAME=value` prefixes become
/// private settings. Chains, pipes and redirects are refused: the program is
/// started directly, never through a shell.
pub fn program_from_command_line(text: &str) -> Result<ServerDraft, ImportProblem> {
    let analysis = crate::terminal::analyse(text);
    if !analysis.is_clear() {
        return Err(ImportProblem::NotOneProgram);
    }
    let [simple] = analysis.commands.as_slice() else {
        return Err(if analysis.commands.is_empty() {
            ImportProblem::Empty
        } else {
            ImportProblem::NotOneProgram
        });
    };
    if !simple.redirects.is_empty() || simple.joiner != crate::terminal::Joiner::Sequence {
        return Err(ImportProblem::NotOneProgram);
    }
    let mut words = simple
        .wrappers
        .iter()
        .flat_map(|wrapper| wrapper.split(' ').map(str::to_owned));
    let mut all: Vec<String> = words.by_ref().collect();
    all.extend(simple.words.iter().map(|word| word.text.clone()));
    let mut parts = all.into_iter();
    let command = parts.next().ok_or(ImportProblem::Empty)?;
    let env = simple
        .assignments
        .iter()
        .filter_map(|assignment| {
            let (name, value) = assignment.split_once('=')?;
            Some((name.to_owned(), value.to_owned()))
        })
        .collect();
    Ok(ServerDraft {
        name: String::new(),
        enabled: true,
        setup: DraftSetup::Program {
            command,
            args: parts.collect(),
            env,
            working_dir: None,
        },
    })
}

/// Package runners: the word after them (and their own flags) names what runs.
const RUNNERS: &[&str] = &[
    "npx", "bunx", "pnpx", "uvx", "pipx", "deno", "docker", "podman",
];

/// A friendly name for a program: `npx -y @modelcontextprotocol/server-filesystem`
/// → `Filesystem`; `uvx mcp-server-git` → `Git`; `/opt/tools/notes-mcp` → `Notes`.
pub fn name_for_program(command: &str, args: &[String]) -> String {
    let program = crate::terminal::program_name(command);
    let interpreter = matches!(
        program.as_str(),
        "python" | "python3" | "py" | "node" | "bun" | "ruby" | "java" | "dotnet" | "uv"
    );
    let subject = if interpreter
        && let Some(module) = args
            .iter()
            .position(|arg| arg == "-m")
            .and_then(|index| args.get(index + 1))
    {
        module.as_str()
    } else if interpreter
        || RUNNERS.contains(&program.as_str())
        || matches!(program.as_str(), "pnpm" | "yarn")
    {
        args.iter()
            .filter(|arg| !arg.starts_with('-'))
            .find(|arg| !matches!(arg.as_str(), "run" | "dlx" | "exec" | "x" | "--"))
            .map_or(program.as_str(), String::as_str)
    } else {
        program.as_str()
    };
    friendly_package_name(subject)
}

fn friendly_package_name(package: &str) -> String {
    // `@scope/name@1.2.3` → `name`; `ghcr.io/org/image:tag` → `image`.
    let without_version = package
        .strip_prefix('@')
        .map_or(package, |rest| rest)
        .split('@')
        .next()
        .unwrap_or(package);
    let base = without_version
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(without_version)
        .split(':')
        .next()
        .unwrap_or(without_version);
    let base = base
        .trim_end_matches(".py")
        .trim_end_matches(".js")
        .trim_end_matches(".exe");
    let mut core = base.to_ascii_lowercase();
    for prefix in ["mcp-server-", "server-", "mcp-"] {
        if let Some(rest) = core.strip_prefix(prefix) {
            core = rest.to_owned();
        }
    }
    for suffix in [
        "-mcp-server",
        "_mcp_server",
        "-mcp",
        "_mcp",
        "-server",
        "_server",
    ] {
        if let Some(rest) = core.strip_suffix(suffix) {
            core = rest.to_owned();
        }
    }
    let words: Vec<String> = core
        .split(['-', '_', '.'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect();
    if words.is_empty() {
        "Program".to_owned()
    } else {
        words.join(" ")
    }
}

/// Id for a program service: stable for the same program and options.
pub fn program_id_for(command: &str, args: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    for arg in args {
        hasher.update([0]);
        hasher.update(arg.as_bytes());
    }
    let digest = hasher.finalize();
    format!("prg-{}", super::hex_of(&digest[..6]))
}

/// The exact command, quoted so spaces and quotes are unambiguous. Shown in full
/// on the review card, never truncated.
pub fn program_display(command: &str, args: &[String]) -> String {
    std::iter::once(command)
        .chain(args.iter().map(String::as_str))
        .map(quote_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./:@=+,~%".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// Fingerprint of everything that decides what a program does. Approval is
/// remembered against it, so any change to the command, options, settings names
/// or folder asks again.
pub fn approval_fingerprint(transport: &McpTransport) -> Option<String> {
    let McpTransport::Program {
        command,
        args,
        env_names,
        working_dir,
    } = transport
    else {
        return None;
    };
    let mut hasher = Sha256::new();
    for part in std::iter::once(command)
        .chain(args)
        .chain(env_names)
        .chain(working_dir)
    {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    Some(super::hex_of(&hasher.finalize()))
}

/// Plain-English facts shown before a program first runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgramNote {
    /// Every program: runs with the person's own access.
    RunsAsYou,
    /// Fetches code from the internet each time (`npx -y`, `uvx`, `docker run`).
    DownloadsCode,
    /// Starts a container.
    RunsContainer,
    /// Asks for administrator rights.
    NeedsAdmin,
    /// The program is a shell running a command line.
    RunsShellCommand,
    /// Receives private settings the person entered.
    GetsPrivateSettings,
}

impl ProgramNote {
    pub const fn text(self) -> &'static str {
        match self {
            Self::RunsAsYou => "Runs on this computer with the same access to your files as you.",
            Self::DownloadsCode => "Downloads a program from the internet and runs it.",
            Self::RunsContainer => "Starts a container on this computer.",
            Self::NeedsAdmin => "Asks for administrator rights.",
            Self::RunsShellCommand => "Runs a shell command, which could do anything you can do.",
            Self::GetsPrivateSettings => "Receives the private settings you entered.",
        }
    }

    /// Whether the review card should highlight this as a warning.
    pub const fn is_warning(self) -> bool {
        matches!(
            self,
            Self::DownloadsCode | Self::NeedsAdmin | Self::RunsShellCommand
        )
    }
}

/// What the review card says about a program, warnings first.
pub fn program_notes(transport: &McpTransport) -> Vec<ProgramNote> {
    let McpTransport::Program {
        command,
        args,
        env_names,
        ..
    } = transport
    else {
        return Vec::new();
    };
    let program = crate::terminal::program_name(command);
    let mut notes = Vec::new();
    match program.as_str() {
        "sudo" | "doas" | "su" | "runas" => notes.push(ProgramNote::NeedsAdmin),
        "sh" | "bash" | "zsh" | "fish" | "cmd" | "powershell" | "pwsh" => {
            notes.push(ProgramNote::RunsShellCommand);
        }
        "docker" | "podman" => {
            notes.push(ProgramNote::RunsContainer);
            if args.first().is_some_and(|arg| arg == "run") {
                notes.push(ProgramNote::DownloadsCode);
            }
        }
        "npx" | "bunx" | "pnpx" | "uvx" => notes.push(ProgramNote::DownloadsCode),
        "pipx" | "pnpm" | "yarn"
            if args.iter().any(|arg| matches!(arg.as_str(), "run" | "dlx")) =>
        {
            notes.push(ProgramNote::DownloadsCode);
        }
        "deno" if args.iter().any(|arg| arg.starts_with("https://")) => {
            notes.push(ProgramNote::DownloadsCode);
        }
        _ => {}
    }
    if !env_names.is_empty() {
        notes.push(ProgramNote::GetsPrivateSettings);
    }
    notes.push(ProgramNote::RunsAsYou);
    notes.sort_by_key(|note| !note.is_warning());
    notes
}

/// Names a model can call: `connected_<service>_<tool>`, shortened with a stable
/// hash when it would pass [`MAX_TOOL_NAME`], so long names never collide or
/// get rejected.
pub fn bounded_tool_alias(alias: String) -> String {
    if alias.len() <= MAX_TOOL_NAME {
        return alias;
    }
    let digest = Sha256::digest(alias.as_bytes());
    let suffix = super::hex_of(&digest[..4]);
    let keep = MAX_TOOL_NAME - suffix.len() - 1;
    format!("{}_{suffix}", &alias[..keep])
}

/// Headers Sovra sets itself; a service's saved headers may not replace them.
const RESERVED_HEADERS: &[&str] = &[
    "accept",
    "connection",
    "content-length",
    "content-type",
    "host",
    "mcp-method",
    "mcp-name",
    "mcp-protocol-version",
    "mcp-session-id",
    "origin",
    "transfer-encoding",
];

/// Whether a saved header may be sent: a valid header name that Sovra does not
/// set itself, with a value that cannot smuggle in another header.
pub fn header_allowed(name: &str, value: &str) -> bool {
    let name_ok = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        && !RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str());
    name_ok && !value.contains(['\r', '\n', '\0'])
}

/// Whether `name` can be an environment variable name on every platform.
pub fn env_name_allowed(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Counts for the list header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct McpSummary {
    pub total: usize,
    pub working: usize,
    pub needs_you: usize,
    pub paused: usize,
}

impl McpService {
    /// Whether the model may call `tool`.
    pub fn tool_is_on(&self, tool: &str) -> bool {
        !self.disabled_tools.iter().any(|name| name == tool)
    }

    /// How many of this service's tools the model may call.
    pub fn tools_on(&self) -> usize {
        self.tools
            .iter()
            .filter(|tool| self.tool_is_on(&tool.name))
            .count()
    }

    /// Whether this service is waiting for the person (sign-in, a detail, an OK).
    pub fn needs_person(&self) -> bool {
        self.enabled
            && matches!(
                self.phase,
                McpPhase::NeedsSignIn
                    | McpPhase::NeedsClientDetails
                    | McpPhase::NeedsApproval
                    | McpPhase::Failed
            )
    }

    /// The line under the name: the web address or the exact program.
    pub fn location(&self) -> String {
        match &self.transport {
            McpTransport::Web => self.address.clone(),
            McpTransport::Program { args, .. }
                if args.len() == 1 && args[0] == SAMPLE_SERVER_FLAG =>
            {
                "Built into Sovra".to_owned()
            }
            McpTransport::Program { command, args, .. } => program_display(command, args),
        }
    }
}

impl McpState {
    /// Header counts.
    pub fn summary(&self) -> McpSummary {
        let mut summary = McpSummary {
            total: self.services.len(),
            ..McpSummary::default()
        };
        for service in &self.services {
            if !service.enabled {
                summary.paused += 1;
            } else if service.phase == McpPhase::Connected {
                summary.working += 1;
            } else if service.needs_person() {
                summary.needs_you += 1;
            }
        }
        summary
    }

    /// Services whose name, address, program or tools contain `query`, ignoring
    /// case; services that need the person come first, then by name.
    pub fn matching(&self, query: &str) -> Vec<&McpService> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<&McpService> = self
            .services
            .iter()
            .filter(|service| {
                query.is_empty()
                    || service.name.to_lowercase().contains(&query)
                    || service.location().to_lowercase().contains(&query)
                    || service
                        .tools
                        .iter()
                        .any(|tool| tool.name.to_lowercase().contains(&query))
            })
            .collect();
        found.sort_by_key(|service| (!service.needs_person(), service.name.to_lowercase()));
        found
    }
}

#[cfg(test)]
mod tests;
