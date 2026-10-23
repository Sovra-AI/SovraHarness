# SovraHarness

> **Closed source — proprietary to Sovra.** Copyright © 2026 Sovra. All rights
> reserved. Do not copy, distribute, or disclose outside Sovra. See [LICENSE](LICENSE).

Native Rust desktop app for the Sovra agent harness. GPUI (via `gpui-kit`) UI, tokio runtime,
embedded SurrealDB memory (vector + graph + full-text in one engine, behind a swappable
`MemoryStore` port), Velopack installers with auto-update. macOS, Windows, Linux from one
codebase.

## Quick start

```sh
# 1. Rust via https://rustup.rs — the pinned toolchain (1.99.1) installs itself on first build
# 2. Linux only: ./scripts/install-linux-deps.sh
cargo xtask setup    # once: install pinned development tools
cargo xtask dev      # start only if the existing sovraRust watcher is absent
```

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

The **My AI's & Routing Tree** tab configures how Sovra selects models across its **22 `SovraSRX` specialist agents**:
- **Single Global Model:** Pin one model from OpenRouter, Ollama, Anthropic, Gemini, or any OpenAI-compatible server across every agent.
- **Per-Agent Models:** Assign individual models or complexity tiers (`Tier 0 · Fast`, `Tier 1 · Standard`, `Tier 2 · Deep`) per specialist (`CodingAgent`, `WebSearchAgent`, `DeepResearchAgent`, `StockMarketAgent`, etc.).
- **Dynamic Tree Router (`CogniRouteSLM` + `SovraSRX`):** Evaluates prompts through a complexity decision tree using **Laya** (`/v1/systemone`), **OpenJev / TypeSafe Jev** (`/v1/systemone` or OpenRouter `/api/alpha/decisions`), or the zero-latency **Embedded Rust Heuristic** engine with automatic offline fail-open and auditable `rc1-...` route codes.

The **Chat** and **Schedules** tabs provide the native **AG-UI (`adk-ui`)** conversation workspace (`"Built around you."` hero banner, live `"Building your personal AI..."` progress checklist, `My Personal AI` right inspector, and type-ahead prompt queue) alongside the `SovraSRX` **Schedules Command Center**.

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
research sources are documented in [`docs/styling.md`](docs/styling.md). The welcome
screen's appearance control previews both modes (session only).
