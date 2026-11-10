# Sovereignty evidence

A verification register for security, data-protection and architecture
teams. Each claim names the code or document that supports it, so it can be
checked independently. Paths are relative to the repository root. Line
numbers drift; search for the named symbol if a line has moved.

Baseline checked on 2026-11-06; group networking and storage updated on 2026-11-10.

## 1. Where data lives

The app data folder is chosen with
`ProjectDirs::from("com", "Sovra", "SovraHarness")` and `data_local_dir()`
(`crates/sovra-app/src/main.rs`). On Linux this is typically
`~/.local/share/sovraharness`.

| Item | Location | Evidence |
| --- | --- | --- |
| Memory database (SurrealDB, SurrealKV engine) | `memory/` | `crates/sovra-app/src/main.rs`; `Cargo.toml` (surrealdb with `kv-surrealkv`) |
| Group records and device identity | `groups/`; device key in credential store | `crates/sovra-app/src/main.rs`; `crates/sovra-mesh/src/lib.rs` |
| Logs (daily rolling) | `logs/` | `crates/sovra-app/src/logging.rs` |
| Model connections and catalogue cache | `models/` | `crates/sovra-providers/src/storage.rs` |
| File index | `file-index/` | `crates/sovra-app/src/main.rs` |
| Chat attachments (copied, owner-only) | `attachments/` | `docs/files.md` |
| Skills, schedules, terminal and connected-service settings | `skills/`, `schedules.json`, `terminal/`, `connected-services/` | `crates/sovra-app/src/main.rs` |
| WhatsApp session keys (if linked) | `whatsapp/` | `crates/sovra-app/src/main.rs` |
| API keys and secrets | OS keychain (`keyring`) | `crates/sovra-providers/src/storage.rs`; `docs/connected-services.md` |
| Development-only key files | `dev-credentials/`, only in debug builds with `SOVRA_DEV=1` | `AGENTS.md` ("Never"); `crates/sovra-app/src/main.rs` |
| Browser previews of HTML/SVG/Mermaid artifacts | OS temp folder `sovra-artifact-views` | `crates/sovra-runtime/src/lib.rs` |

The memory store's remote (`ws://`) mode is not compiled in, so it cannot
connect remotely (`crates/sovra-store-surreal/src/lib.rs`; workspace
`Cargo.toml` features).

**Verify yourself:** look at the folder above; run
`grep -rn "sentry\|posthog\|telemetry\|analytics" crates/ Cargo.toml`. The
only hit is a PowerShell telemetry **opt-out** in the terminal adapter
(`crates/sovra-core/src/terminal/mod.rs`).

## 2. Every network destination

The complete outbound inventory. "Content" means prompts, files, memory or
messages.

| Destination | When | Default? | Content sent | Off switch |
| --- | --- | --- | --- | --- |
| `openrouter.ai/api/v1/models` (model list) | At launch, if the cached list is over 15 minutes old | **Yes**: OpenRouter is a default connection (`crates/sovra-providers/src/storage.rs`, `SavedModels::default`) | No (an API key only if one is saved) | Delete the OpenRouter connection on the Models page |
| `openrouter.ai/api/v1` (chat) | Person picks an OpenRouter model | No | Yes | Do not use it |
| `api.anthropic.com` | Person adds an Anthropic connection | No | Yes | Do not add it |
| Any OpenAI-compatible URL (default `localhost:1234/v1`) | Person adds the connection | No | Yes, to that URL | Keep it on localhost |
| `localhost:11434` (Ollama) | Model list at launch; chat | Yes (local) | Local only | Delete the connection |
| Ollama model download (`api/pull`) | Person downloads a model in the app | No | No; the Ollama program fetches from its own registry | Download offline and copy in |
| Router at `127.0.0.1:8011` (Laya) | Every chat prompt | **Yes**, but a **local address** | The prompt, to the local address | Choose the built-in router (not saved between launches), or leave nothing listening (1.2 s timeout, silent fallback) |
| `openrouter.ai/api/alpha/decisions` (OpenJev router) | Only if selected | No | The prompt | Do not select it |
| Gemini (`generativelanguage.googleapis.com`) | Only if `GEMINI_API_KEY` / `GOOGLE_API_KEY` is set | No | Yes | Leave unset |
| `api.mwmbl.org`, `en.wikipedia.org` (web search) | When a helper searches the web | **Yes**: `web_search_enabled: true` (`crates/sovra-core/src/models.rs`, `AgentSettings::default`) | The search query, written from the prompt | Turn off web search in agent settings |
| Public web pages | When a helper opens a search result | With web search | The URL only; private and loopback addresses are blocked | Turn off web search |
| Self-hosted SearXNG | Only if `SOVRA_SEARXNG_URL` is set | No | Search query | Leave unset |
| `api.telegram.org` | Background poll once a bot is linked | No | Yes | Unlink |
| WhatsApp Web servers | Persistent connection once paired | No | Yes | Unlink |
| Google sign-in and APIs (Gmail, Calendar, Drive, People, Tasks) | Once connected; tool use | No | Drafts, events, tasks, read files | Disconnect Google |
| Group peers and n0 discovery/relays | Group transport starts with runtime when device credentials are available | Yes, Internet reach by default | Encrypted shared records to peers; discovery/relay metadata | `SOVRA_GROUP_REACH=local` removes Internet discovery/relay; direct peer traffic remains possible |
| Remote MCP servers | Reconnect at launch once added | No | Tool arguments | Disable or remove the service |
| `cdn.jsdelivr.net` (Mermaid script) | The **browser** loads it when a diagram is opened in the browser | No | No; diagram source stays in the local page | Do not open diagrams in the browser |

**Not found anywhere in the code:** telemetry or analytics, crash reporting,
licence checks, remote font or asset loading, an update check. The Velopack
call at startup only handles install hooks and makes no network request
(`crates/sovra-app/src/main.rs`).

## 3. Fully local setup (shipped options only)

To run with **no traffic beyond the computer itself**:

1. Install a local model server: Ollama, or an OpenAI-compatible server such
   as LM Studio, llama.cpp or vLLM. Load a model onto it.
2. On the **Models** page, delete the default **OpenRouter** connection and
   keep only the local one.
3. Turn **web search** off in the helpers' settings.
4. Choose the built-in router after each launch, or run nothing at
   `127.0.0.1:8011`. Both stay on the computer either way.
5. Leave `GEMINI_API_KEY`, `GOOGLE_API_KEY` and `SOVRA_SEARXNG_URL` unset.
6. Do not link WhatsApp, Telegram, Google or remote add-on tools.
7. If search by meaning is wanted, point it at the local Ollama.
8. Set `SOVRA_GROUP_REACH=local` before launch and do not join or create a group.
   Local reach alone can still communicate with other computers directly.

These settings avoid the listed external services. Verify actual egress in the
deployment environment before claiming a network-isolated installation. This is
**not the default**, and it has not been packaged as a tested offline or air-gapped edition ([08](08-limits-and-objections.md)).

## 4. Knowledge integrity

| Claim | Evidence |
| --- | --- |
| Facts are never edited or deleted; four time fields plus `stopped_at` | `crates/sovra-core/src/memory.rs` (module header, `Fact`) |
| Each fact quotes an exact substring of its source; invented quotes reject the batch | `docs/memory-capture-and-recall.md` ("quote guard") |
| Corrections supersede; history is readable as of a past date | `docs/memory-capture-and-recall.md`; `crates/sovra-store-surreal/src/facts.rs` |
| Stop a fact, a chain or a whole source; nothing deleted | `docs/memory-capture-and-recall.md` |
| Remembering can be switched off; strangers and connected-account chats are not remembered | `docs/memory-capture-and-recall.md`; `docs/communications.md` |
| Recall: word search, optional meaning search, raw text; rank fusion; two-second deadline; honest fallback message | `crates/sovra-runtime/src/recalling.rs` (`RECALL_DEADLINE`); `crates/sovra-core/src/memory.rs` |
| Recalled memory is marked as untrusted data and capped in size | `docs/memory-capture-and-recall.md` |
| Search by meaning is off by default; the chooser shows the destination | `docs/agent-memory.md`; `crates/sovra-core/src/memory/semantic.rs` |
| AI-proposed file links need a person's **Save**; confidence shown as uncalibrated | `docs/skills-and-file-knowledge.md`; `crates/sovra-ui/src/knowledge_review.rs` |
| Saved work keeps versions; delete is soft and asks twice | `crates/sovra-core/src/artifacts.rs`; `crates/sovra-ui/src/artifacts_view.rs` |
| Artifacts are never executed inside the app | `docs/teams-and-artifacts.md` |

## 5. Human oversight

| Claim | Evidence |
| --- | --- |
| Commands: Allow once / Always / Don't allow; risky forms always ask and are never remembered; rules are text matches, not a sandbox | `docs/terminal.md` |
| Messages: review card with exact text; no model tool approves or edits; ten-minute deadline; no auto-retry; receipts | `docs/communications.md` |
| Scheduled messages: consent at creation (who, app, words, time); no review when they fire | `docs/communications.md`; `crates/sovra-core/src/schedules.rs` |
| Strangers get no memory, files, skills, services or tools | `docs/communications.md` |
| Gmail drafts only; Calendar `sendUpdates=none`; Drive read-only | `docs/google.md` |
| MCP programs need **Run it**; any change re-asks; no per-call approval yet; not sandboxed | `docs/connected-services.md` |

## 6. Engineering controls

| Claim | Evidence |
| --- | --- |
| No `unsafe` code (forbidden by workspace lints) | `Cargo.toml` `[workspace.lints]`; `AGENTS.md` |
| Layering checked in CI: pure core, adapters, UI | `xtask/src/deps.rs`; `docs/architecture.md` |
| Licence and advisory checks; OpenSSL banned (rustls only) | `deny.toml`; `AGENTS.md` |
| Secrets in keychain; development exception confined to debug builds with `SOVRA_DEV=1` | `AGENTS.md`; `crates/sovra-app/src/main.rs` |

## 7. Evaluation evidence

Dated evaluation outputs are kept in `docs/evaluations/`. They used
**synthetic data**, and the documents state they are not benchmarks
(`docs/agent-memory.md`). Highlights:

- Recall ranking reached 576/576 expected results at 100k sources after the
  final ranking design (`memory-recall-2026-11-03`).
- Word search p95 1.7 ms at 5,000 facts (`memory-2026-10-27`).
- No LongMemEval run yet.
