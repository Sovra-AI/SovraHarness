# SovraHarness

> **Closed source — proprietary to Sovra.** Copyright © 2026 Sovra. All rights
> reserved. Do not copy, distribute, or disclose outside Sovra. See [LICENSE](LICENSE).

Native Rust desktop app for the Sovra agent harness. GPUI (via `gpui-kit`) UI, tokio runtime,
embedded SurrealDB memory (vector + graph + full-text in one engine, behind a swappable
`MemoryStore` port), Velopack installers with auto-update. macOS, Windows, Linux from one
codebase.

## Quick start

```sh
just dev
```

Or without `just`:

```sh
cargo xtask setup
cargo xtask dev
```

On Linux, install system packages once first with `./scripts/install-linux-deps.sh`.
That's it — no Node, no Python, no system packages on macOS/Windows.

## Everyday commands

| Command                  | What it does                                                 |
| ------------------------ | ------------------------------------------------------------ |
| `cargo xtask dev`        | Persistent live reload: watch, rebuild/relaunch, stream logs  |
| `cargo xtask run`        | Launch the app (debug)                                       |
| `cargo xtask ci`         | Every gate: fmt, clippy, layering, cargo-deny, unused deps, tests |
| `cargo xtask fix`        | Auto-apply fmt + clippy fixes                                |
| `cargo xtask check-styles` | Check native styling boundaries and the bundled SVG logo  |
| `cargo xtask setup`      | Install pinned CI tools and Watchexec                       |
| `cargo xtask dist`       | Release build (+ installers if Velopack `vpk` is installed)  |

Logs stream directly in the dev pane, with files under the OS data dir
(`~/Library/Application Support/com.Sovra.SovraHarness/logs` on macOS). The runner
preserves an existing `RUST_LOG` filter and defaults to focused application debug logs.

For development, use the existing **`sovra:sovraRust`** tmux window: the watcher and
live logs stay running there. Save Rust or embedded asset changes to rebuild and
relaunch automatically; no manual start/stop is needed. This resets in-memory
state rather than hot-swapping Rust code. Setup and operating rules:
[`docs/development.md`](docs/development.md).

The **Models** tab includes the complete OpenRouter catalog, Ollama discovery and
downloads, custom local/remote connections, favorites, comparison and composable
filters. Setup, authentication and capability details: [Model library](docs/models.md).

The **My AI's** tab lets anyone choose how Sovra picks an AI, in plain language:
- **Automatic (recommended):** a fast decision engine (**Laya**, **OpenJev / TypeSafe Jev**,
  or the zero-latency **Embedded Rust Heuristic**, with offline fail-open and auditable
  `rc1-...` route codes) sends simple questions to a quick model and hard ones to a
  powerful one.
- **One AI for everything:** use the model picked in **Models** for every task.
- **Choose per task:** set effort (Quick / Balanced / Thorough) for each of the 22
  `SovraSRX` specialists.

The engine choice, routing tree and prompt tester are folded under
*How automatic choice works → Show details*.

**Settings** is the one place to set Sovra up, written for people new to AI:
**Appearance** (Light or Dark), **AI connection** (follow the three steps to paste an
OpenRouter key; it is kept in the OS keychain and can be checked, replaced or removed)
and a folded-away **Advanced** section for local or other services, whose technical
fields stay hidden until you choose *Show technical options*. Then select a chat
model in **Models**.

**Chat** starts with a normal conversation using the selected model. Its look follows SovraSRX's chat:
gradient user bubbles, bare replies beside the Sovra avatar and a glass composer. While
Sovra works, a "Sovra is thinking…" pill shows the elapsed time; models that reason
stream their thinking into a collapsible panel that folds when the answer starts. Replies render
as selectable Markdown with code copying. Each new reply includes a model pill
with its connection and exact model ID on hover; this identity is saved with the
conversation and stays accurate after switching models. Streaming follows the latest output;
scrolling up pauses following until you return to the bottom or use **Jump to latest**.
Visible model reasoning appears in a collapsible panel with an elapsed-time indicator.
Enter sends and Shift+Enter adds a line. **Stop** also discards queued prompts.
Native AG-UI cards and buttons are optional tools for interactive requests, and
their actions return to the same session with the surface ID and revision.
Routing simulation remains in **My AI's**. The **Schedules** tab retains the
`SovraSRX` Schedules Command Center.

## Shipping

Push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds each OS, signs + notarizes (when the
secrets listed in that file are set), produces Velopack installers (`.pkg` on macOS,
`Setup.exe` on Windows, AppImage on Linux) and publishes a GitHub Release that installed apps
auto-update from.

## For contributors and AI agents

Workflow and best practices: [`CONTRIBUTING.md`](CONTRIBUTING.md). Canonical rules:
[`AGENTS.md`](AGENTS.md) (read natively by Codex and Cursor; imported by `CLAUDE.md`
for Claude Code), plus a short `AGENTS.md` per crate. Reasoning: [`docs/architecture.md`](docs/architecture.md).
Edits are checked automatically by `cargo xtask hook` (Claude Code + Cursor hooks) and by CI.
Every commit uses gitmoji and must update [`CHANGELOG.md`](CHANGELOG.md) under **Unreleased**
before pushing to `main` (enforced by [`AGENTS.md`](AGENTS.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md));
security and secrets policy lives in [`SECURITY.md`](SECURITY.md).

The native Sovra dark/light design system, embedded SVG branding, usage examples and
research sources are documented in [`docs/styling.md`](docs/styling.md). Switch
between both modes in **Settings → Appearance**; the choice is remembered.
