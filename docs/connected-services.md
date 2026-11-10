# MCP servers (connected services)

How extra tools are added from a web address, a program on this computer, or
pasted setup, including the built-in sample server.

In the app this is the **MCP servers** card in **Connections** (the button is **Add MCP
server**). The code calls them "services".

## The built-in sample

`sovra --sample-mcp-server` (`sovra-runtime/src/mcp/sample.rs`) is a small stdio MCP
server inside Sovra's own binary. Its tools are harmless and use no files and no
network:

- `roll_dice` — returns the number as text and as a native AG-UI card (`structuredContent`
  with `protocol: "ag_ui"`)
- `tell_the_time`
- `make_a_checklist`
- `count_words`

**Try the sample MCP server** (`McpCommand::AddSample`) adds it as a program pointing
at the running executable. It is approved automatically: it is Sovra itself, so there
is nothing to review. Developers can also point any MCP client at it as a test server.
For a full-featured third-party test server, use `npx -y
@modelcontextprotocol/server-everything`.

Connected services give the assistant tools from outside Sovra through the
[Model Context Protocol](https://modelcontextprotocol.io). A person can add as many
as they like (up to `MAX_SERVICES`, 100) and manage each one from its own card in
**Connections → A service you already use**.

## Kinds of service

| Kind | Transport | How it signs in | Where its secrets live |
| ---- | --------- | --------------- | ---------------------- |
| Web address | Streamable HTTP (hand-written client in `sovra-runtime/src/mcp.rs`) | OAuth 2.1 + PKCE with discovery and dynamic registration, or a static secret, or saved headers | Keychain (`SecretKind::Access`, `Static`, `Settings`) |
| Program on this computer | stdio, via the `rmcp` client (`sovra-runtime/src/mcp/programs.rs`) | Environment variables, per the MCP spec | Keychain (`SecretKind::Settings`) |

The table describes installed/release storage. Debug `SOVRA_DEV=1` uses the
same separate development credential files as model connections; see
[Configuration](configuration.md). Only names (environment variable names, header
names) appear in the saved list, events and logs. Secret values are loaded from
the credential store for the program or exact web address they were saved for.

## Adding

- **Web address**: name, address and optional secret. `https`, or `http` for
  `localhost` only.
- **Program**: the command line exactly as instructions give it. `sovra-core` parses
  it with the terminal tool's shell reader (`terminal::analyse`), so quotes work. It
  refuses chains, pipes, redirects and `$(…)`, because programs are started directly,
  never through a shell. `NAME=value` prefixes and the **Private settings** box become
  environment variables.
- **Paste settings** (`parse_server_settings`) reads:
  - `{"mcpServers": {…}}` from Claude Desktop and Cursor;
  - `{"servers": {…}}` from VS Code;
  - `{"context_servers": {…}}` from Zed, including its nested `command.path`;
  - `{"mcp": {"servers": …}}`;
  - a single server object;
  - a bare name → server map;
  - a web address;
  - a command line.

  Entries marked `"disabled": true` arrive paused. Reserved headers are dropped:
  `Content-Type`, `Mcp-*` and `Origin`, so saved headers can't override what Sovra
  sets.

## Before a program runs

The MCP security best practices require local-server consent: show the exact
command without truncation, say that it executes code, and require explicit
approval. So a new program arrives as `McpPhase::NeedsApproval`. Its card shows:

- the full quoted command (`program_display`);
- the private-setting names and the folder it starts in;
- plain notes from `program_notes`, warnings first:
  - "Downloads a program from the internet and runs it" (`npx`, `uvx`, `docker run`, …);
  - "Runs a shell command…";
  - "Asks for administrator rights";
  - "Runs on this computer with the same access to your files as you".

**Run it** records an `approval_fingerprint`: a SHA-256 of the command, arguments,
setting names and folder. Programs whose fingerprint still matches start
automatically on later launches. Any change to those asks again.

## Running many services well

- **Nothing blocks the actor.**
  - Tool calls run in their own tasks, for web addresses and programs alike.
  - Each sign-in has its own waiter task, so several sign-ins can be open at once.
  - Each program has its own task and request queue.
  - Reconnects at start-up and after an import are queued one per message, so screen
    commands and tool calls interleave between them.
- **Programs**:
  - Each starts in its own process group with the login shell's environment (shared
    with the terminal tool), so `npx`/`uvx` installed through shell profiles are found.
  - On Windows, `npx` resolves to `npx.cmd` through `PATHEXT`.
  - The last 12 error-output lines are kept. When a program quits or fails to start,
    its card says what it said.
  - Pausing, removing or quitting stops the whole group.
  - First start allows two minutes (package downloads); each tool call allows two
    minutes.
- **Web addresses**:
  - On start-up they reconnect with the saved sign-in or secret.
  - An expired sign-in shows **Sign in** instead of failing.
  - Saved headers and tokens go only to the exact address they were saved for.
- **Tools**:
  - Switched-off tools (`disabled_tools`) never reach the model, and calls to them are
    refused.
  - Tool names are `connected_<service>_<tool>`, shortened with a stable hash past 64
    characters, so OpenAI and Gemini accept them.
- **The saved list** lives at `<app data>/connected-services/services.json`, written by
  write-then-rename. Older builds kept it as one keychain entry, which can't grow on
  Windows (about 2.5 KB per credential). It is moved to the file on first load.
- **The screen**:
  - a summary line ("3 working · 1 need you · 1 paused");
  - search across names, addresses and tool names once the list has more than four
    services;
  - services that need the person sorted first;
  - tool lists folded behind "N of M tools on";
  - Remove asks once more in place.

## Where the code is

| Concern | File |
| ------- | ---- |
| Address rules, sign-in rules, protocol messages, state, copy | `crates/sovra-core/src/mcp.rs` |
| Programs, pasted settings, review notes, fingerprint, search, summary | `crates/sovra-core/src/mcp/servers.rs` |
| Actor: web client, sign-in, list file, commands | `crates/sovra-runtime/src/mcp.rs` |
| Program tasks | `crates/sovra-runtime/src/mcp/programs.rs` |
| Tool exposure to the model | `crates/sovra-adk/src/remote_tools.rs` |
| Screen | `crates/sovra-ui/src/service_connect_view.rs` |

## Not yet

- No approval card per tool call. The person controls which tools are on instead.
- Tool annotations (read-only/destructive hints) are not used.
- Resources, prompts, sampling and elicitation are not supported.
- The legacy HTTP+SSE transport is not supported.
- Programs are not sandboxed beyond the person's own rights. The review card says so.
