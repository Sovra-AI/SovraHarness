# SovraHarness

> **Closed source — proprietary to Sovra.** Copyright © 2026 Sovra. All rights
> reserved. Do not copy, distribute, or disclose outside Sovra. See [LICENSE](LICENSE).

Native Rust desktop app for the Sovra agent harness. GPUI (via `gpui-kit`) UI, tokio runtime,
embedded SurrealDB memory (vector + graph + full-text in one engine, behind a swappable
`MemoryStore` port), Velopack installers and update hooks. macOS, Windows, Linux from one
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
| `cargo xtask dist`       | Release build and verified native installer; requires Velopack `vpk` 1.2.161 |

Logs stream directly in the dev pane, with files under the OS data dir
(`~/Library/Application Support/com.Sovra.SovraHarness/logs` on macOS). The runner
preserves an existing `RUST_LOG` filter and defaults to focused application debug logs.

For development, use the existing **`sovra:sovraRust`** tmux window: the watcher and
live logs stay running there. Save Rust or embedded asset changes to rebuild and
relaunch automatically; no manual start/stop is needed. This resets in-memory
state rather than hot-swapping Rust code. Setup and operating rules:
[`docs/development.md`](docs/development.md).

Development builds use a separate plaintext key store in the app data directory
when `SOVRA_DEV=1` is set by the runner. Release builds always use the OS keychain.
See the development guide for storage and permission details.

The **Models** tab starts with guided setup: connect your account, choose an AI,
then go to Chat. When available, it offers an explicitly free online model to try.
**Browse all models** opens the complete catalog, optional Ollama connections,
favorites and comparison; filters stay closed until needed. Setup, authentication
and capability details: [Model library](docs/models.md).

The **My AI's** tab lets anyone choose how Sovra picks an AI, in plain language:
- **Automatic (recommended):** a fast decision engine (**Laya**, **OpenJev / TypeSafe Jev**,
  or the **Embedded Rust Heuristic**, with offline fail-open and auditable
  `rc1-...` route codes) simulates which model to choose for each task. Chat
  currently uses the model selected in **Models**.
- **One AI for everything:** use the model picked in **Models** for every task.
- **Choose per task:** set effort (Quick / Balanced / Thorough) for each of the 22
  `SovraSRX` specialists.

The engine choice, routing tree and prompt tester are folded under
*How automatic choice works → Show details*.

The **Knowledge** tab explains how Sovra remembers, with one worked example a
new reader can follow: something happens, a record is kept, a current belief is
formed, a later belief replaces it, and the earlier belief stays as history.
The same page names who or what the belief is about, and that finding a memory
uses both the meaning and the words themselves. It is an explanation, not the
chat, and it does not need a memory store to be open. The times Sovra keeps are
under *Show the times Sovra keeps*.

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

For a local installer, install the build tool with
`dotnet tool install -g vpk --version 1.2.161`, then run `cargo xtask dist`.
The command prints the installer path under `target/dist/` and fails if packaging
is unavailable or no installer is produced. `SOVRA_VPK` can point to an isolated
tool installation. Only the native app is packaged: users do not need Rust,
.NET, Node, Python or a separate database to run it. Online AI requires an account
with the chosen service; optional local servers such as Ollama are configured
separately. Local packages must be signed/notarized before wider distribution.

Push a `vX.Y.Z` tag. `.github/workflows/release.yml` builds each OS, signs + notarizes (when the
secrets listed in that file are set), produces Velopack installers (`.pkg` on macOS,
`Setup.exe` on Windows, AppImage on Linux) and publishes a GitHub Release with update
feeds. The app handles Velopack lifecycle hooks; automatic update checking and an
in-app update action are not implemented yet.

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
