# Configuration and local data

The composition root, [sovra-app](../crates/sovra-app/src/main.rs), selects paths,
credentials and startup settings. Most people configure connections in Settings
and Models. Environment changes require restarting the app.

## Startup variables

| Variable | Current use |
| --- | --- |
| `SOVRA_DEV=1` | Debug builds use separate development credentials and start Development mode on. Release builds ignore this exception. `cargo xtask dev` sets it. |
| `OPENROUTER_API_KEY`, `OLLAMA_API_KEY` | Fallback keys for the built-in connections at their default destinations. A saved key takes precedence. |
| `GEMINI_API_KEY`, `GOOGLE_API_KEY` | Agent configuration fallback; Gemini takes precedence. This does not replace the Models setup flow. |
| `SOVRA_GOOGLE_CLIENT_ID`, `SOVRA_GOOGLE_CLIENT_SECRET` | Desktop OAuth client fallback: saved Settings client, then startup environment, then build-time values. See [Google](google.md). |
| `SOVRA_SEARXNG_URL` | Optional additional web-search source. Omitting it does not disable other web search. |
| `SOVRA_MEMORY_EXTRACTION_SLOTS` | Background memory-extraction capacity; parsed at startup. See [memory limits](memory-capture-and-recall.md#limits-in-force). |
| `SOVRA_GROUP_REACH=local` | Direct group networking without Internet discovery/relays. Other values select Internet reach. |
| `RUST_LOG` | Tracing filter for app logs. |
| `RUST_BACKTRACE` | Rust backtraces; the dev launcher defaults it to `1` if unset. |
| `SOVRA_TARGET_BUDGET_GIB` | Advisory build-cache budget for the dev launcher, default 30 GiB. |
| `SOVRA_VPK` | Override the Velopack executable for local packaging; see [releasing](releasing.md). |
| `CARGO_TARGET_DIR` | Cargo build output location; xtask cache reporting honours it. |
| `SHELL`, `PATH` | Inputs to terminal execution setup; see [Terminal](terminal.md). |

Do not put real keys into tracked configuration or examples. Clearing a saved
model key can reveal an environment fallback; remove both to remove access.

## Data location

The app uses `directories::ProjectDirs::from("com", "Sovra", "SovraHarness")`
and its local-data directory. Typical defaults are:

| OS | App data root |
| --- | --- |
| Linux | `$XDG_DATA_HOME/sovraharness`, or `~/.local/share/sovraharness` |
| macOS | `~/Library/Application Support/com.Sovra.SovraHarness` |
| Windows | `%LOCALAPPDATA%\Sovra\SovraHarness\data` |

These are separate from Cargo's `target/`. Clearing build caches does not reset
accounts, memory or connections.

| Path within app data | Contents |
| --- | --- |
| `memory/` | Embedded memory database and persisted workspace records |
| `models/` | Connection metadata, preferences and cached catalogs |
| `attachments/` | Managed chat attachments |
| `skills/` | Imported skill packages |
| `file-index/` | Local folder-index data |
| `whatsapp/` | WhatsApp session database, including pairing credentials |
| `connected-services/services.json` | Saved MCP service catalog |
| `schedules.json` | Scheduled prompts, timed sends and run history |
| `groups/` | Group records and shared state |
| `terminal/settings.json` | Terminal preferences |
| `development-control/` | Local development-control state and endpoint descriptor |
| `dev-credentials/` | Debug-only plaintext credential store |
| `logs/` | Daily rolling application logs |

Installed/release credentials use the OS keychain. Debug `SOVRA_DEV=1` credentials
are separate files with owner-only permissions on Unix. Development and normal
launches otherwise use the same app-data root: a dev launch is not an isolated
copy of your workspace.

## Backup and troubleshooting

Close the app before taking a filesystem copy of its data. Keep backups private:
the database, attachments, schedules, WhatsApp session and development credentials
can contain sensitive information. Keychain entries are separate and do not come
with a directory copy. There is no supported one-click backup/restore or database
migration command; verify recovery separately before relying on a backup across
versions, and keep the original untouched.

For build failures, use the launching terminal and [development guide](development.md).
For connection failures, check the relevant model or service card and its saved
settings. For missing credentials, check whether this launch uses the keychain
or development files. Review logs locally and redact private data before sharing.
For MCP access, follow [Development MCP](development-mcp.md); do not copy endpoint
descriptors or machine-specific client configuration into the repository.
