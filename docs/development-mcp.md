# Test the running app through MCP

How to inspect and operate the running native window from Codex or Claude Code
after a one-shot `cargo xtask dev` launch.

`cargo xtask dev` starts **Development mode** automatically in debug builds with
`SOVRA_DEV=1`. The top bar says
**Development mode** and opens Settings when clicked. Turn it off there to revoke
control immediately, close connections, and remove the local endpoint. The next
manual `cargo xtask dev` starts it on again.

Ordinary binaries and release/packaged builds always **start with mode off**, even
if a previous development run left a saved on preference or `SOVRA_DEV=1` is set
for a release binary. Enable it explicitly in **Settings → Development mode**
when needed in those builds. The stdio bridge cannot enable its own access.
This startup policy does not change credential storage: releases always use the
OS keychain, and all development transport authentication and limits still apply.

This is a standard **stdio MCP server** implemented with the official Rust SDK
(`rmcp`, pinned in Cargo.toml). Start a client connection with the command copied
from Settings. This launches only a small protocol bridge, not another app or
database, and not a file watcher. Keep the GUI from `cargo xtask dev` open with
Development mode enabled before tools can operate it.

## Connect a coding assistant

Replace `/absolute/path/to/sovra` with the executable shown in Settings. Use an
absolute path: the MCP process can have a different working directory from the
checkout. On Windows use the absolute `sovra.exe` path.

For [Codex CLI](https://learn.chatgpt.com/docs/extend/mcp?surface=cli):

```sh
codex mcp add sovra-development -- /absolute/path/to/sovra --development-mcp
```

Or add this server in Codex's MCP settings (the CLI configuration is
`~/.codex/config.toml`; a trusted project may use `.codex/config.toml`):

```toml
[mcp_servers.sovra-development]
command = "/absolute/path/to/sovra"
args = ["--development-mcp"]
startup_timeout_sec = 30
tool_timeout_sec = 15
```

For [Claude Code](https://code.claude.com/docs/en/mcp):

```sh
claude mcp add --transport stdio --scope local sovra-development -- /absolute/path/to/sovra --development-mcp
```

Other MCP clients can use the same command and arguments. A typical JSON
configuration is:

```json
{
  "mcpServers": {
    "sovra-development": {
      "command": "/absolute/path/to/sovra",
      "args": ["--development-mcp"]
    }
  }
}
```

No access token belongs in these configurations. Enable the server's tools in
that client's normal permissions UI. Existing stdio sessions discover the app's
current endpoint for every call, so an app reload needs only a fresh inspection,
not a new client configuration. Calls during a reload return an actionable error.
A changed executable location requires updating the client command.

## Tools and an autonomous testing loop

| Tool | Purpose |
| --- | --- |
| `sovra_inspect` | Current observed native controls: target IDs, accessible labels, roles, logical bounds, visibility, disabled/checked/selected/expanded state. Safe appearance/text-size diagnostics; input values are omitted. |
| `sovra_screenshot` | PNG of the actual current window, resized to fit 1600×1600; renderer support is required. Captures visible content, with the app's normal password masking. |
| `sovra_click` | Native pointer move/down/up at an observed control. Requires a visible, enabled target. |
| `sovra_type` | Text into the focused native input; click the intended input first. |
| `sovra_key` | A GPUI keystroke, e.g. `tab`, `enter`, `escape`, `cmd-a` on macOS or `ctrl-a` where supported. |
| `sovra_scroll` | Native wheel input over an observed container; negative lines scroll down. |

The app has one shared window. Multiple clients can attach, but coordinate one
assistant driving it at a time; do not run input tools in parallel.

1. Inspect and capture a screenshot to establish the starting screen.
2. Choose the full `target` returned by inspection. Scroll the relevant container
   if a control is clipped. Never invent target IDs or reuse them across reloads.
3. Exercise the new feature through real controls, using fixture/test data.
4. Inspect again and capture the resulting screen. Asynchronous work needs bounded
   polling until the expected state appears; `input_delivered` is not completion.
5. Preserve native confirmation flows. Check failure, disabled, and empty states,
   and verify the minimum window and enlarged-text layouts with headless tests.
6. Restore temporary settings/test data and run `cargo xtask ci`.

Example instruction to a coding assistant:

> Use Sovra's development MCP to open Settings, change the appearance, confirm
> the selected control and resulting screenshot, then restore the original
> appearance. Report the observed result and any limitation.

`null` state fields mean the control does not expose that property; do not treat
them as false. Appearance and interface scale have explicit safe diagnostics.

Inspection reports native accessibility metadata, not a DOM or all rendered
text. Custom interactive elements need stable IDs, accessible labels and
`.test_support()` observation. Standard GPUI controls already expose observation.
For text/visual assertions use screenshots and reviewed, explicitly safe state
exposure; do not add a general dump of internal models or credentials. Image
encoding runs off the UI thread. A platform renderer that cannot capture images
returns an error rather than a fabricated screenshot.

## Boundaries and implementation

`development.rs` in core owns the typed request/event contract, validation and
state reducer. The runtime owns the SDK, local IO, lifecycle, persistence and
cancellation. The UI reads events and drives GPUI on its thread, returning replies
as commands over `Client`. The app resolves paths/executable once. No second
consumer steals the UI event stream, and no UI entity is shared with the runtime.
GPUI Kit's pinned `test-support` feature is enabled in the native UI dependency
so observed controls and native screenshots remain available in installed builds;
access to these operations is still gated by the development-mode state.

The stdio bridge uses newline-delimited JSON-RPC as specified by
[MCP transports](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
Stdout is protocol-only. SDK negotiation handles supported protocol versions,
initialization, tool discovery, errors and cancellation. Inspection/screenshot
have read-only tool annotations; input tools conservatively advertise potential
destructive and external effects because clicking controls can perform real work.
UI content is data, not trusted instructions to the connected assistant.

Behind stdio, a private non-HTTP control channel binds **127.0.0.1 only**, on an
OS-assigned port. It requires a fresh 256-bit random credential for each
activation and app process. There is no unauthenticated browser API or remote
listener. The descriptor and preference live in the app data directory's
`development-control/`, outside the repository. Unix directories/files must be
0700/0600; symlinks and public permissions are rejected when read. Other platforms
inherit the current user's app-data permissions. An OS-held owner lock prevents
two apps from publishing competing endpoints. Authentication material is never
included in UI status, protocol replies, or logs.

Limits: 32 KiB incoming frames, 8 MiB replies, 16 simultaneous local connections,
8 pending UI requests, 5-second IO timeout, 3-second UI deadline, 4096-byte typing,
100 scroll lines per request, 1000 observed elements per snapshot (visible controls first, explicitly
marked when truncated), and 5 MiB encoded PNGs. Generation checks reject stale
work across activation/restart; disabling closes the listener and cancels pending
calls. Cancellation/timeouts do not undo an input already delivered: inspect
before retrying any action. No arbitrary code, shell, filesystem, raw domain
command, credential retrieval, or remote mode-enable tool exists.

Development mode grants a connected, same-user local process control over real
app data. It is not a sandbox against other programs running as that OS user.
Use a test account/data for destructive tests. Screenshots and labels may contain
personal content, so connect only assistants you intend to share that content
with. Turning mode off revokes subsequent control; it cannot undo earlier edits
or erase content already received by a client.
