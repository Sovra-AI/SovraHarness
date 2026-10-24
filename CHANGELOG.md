# Changelog

All notable changes to SovraHarness are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Releases are cut by pushing a
`vX.Y.Z` tag (see [README → Shipping](README.md#shipping)); installed apps
auto-update from the resulting GitHub Release via Velopack.

Add entries under **Unreleased** as part of the change that introduces them;
move them under a version heading when the tag is cut.

## [Unreleased]

### Changed

- Restyle the standard chat after SovraSRX: bare assistant replies beside a Sovra
  avatar (ringed while writing), gradient user bubbles with a tail corner and initials
  avatar, responsive 720/768/880px reading column, 16px/1.7 reply text, a glass
  composer with violet focus glow, round send and red stop buttons, a glass "Jump to
  latest" pill and quiet copy actions.
- Show model reasoning: a shimmering "Sovra is thinking…" pill with an elapsed timer
  before output, then a collapsible reasoning panel that streams open, folds when the
  answer starts and reads "Thought for Ns" afterwards (saved with the conversation).
- Make streaming cheaper: history is synced once per turn instead of copying the whole
  transcript on every token; Markdown views receive only appended bytes (parsed off
  the UI thread); thinking chunks are coalesced like answer chunks; collapsed reasoning
  is never built; the per-token fade animation is off and the only continuous
  animation is the pre-answer shimmer.
- Streamed tokens are coalesced per frame (8ms) before they reach the UI, so each
  frame costs at most one state update and one Markdown parse. Measured on a
  20,000-chunk reply after 40 long messages: the core reducer went from 327ms to 0.7ms.
- Render the conversation as its own cached GPUI view: composer cursor blinks,
  typing, the sidebar and the elapsed timer reuse its last frame instead of laying out
  every visible message. Measured with a long conversation on screen: idle CPU after
  a reply fell from ~20% to ~7% of a core (the cost of the cursor blink alone).
  GPUI frame-time overlay, release builds, 40 earlier replies plus a 3,400-token
  streamed reply: frames after the reply drop from 72.8 ms to 3.6 ms, and the slowest
  10% / 1% of frames late in the stream from 71.1 / 74.7 ms to 40.9 / 48.7 ms.
- Highlight code blocks with `gpui-kit`'s `syntect` feature (pure-Rust regexes,
  cached per unchanged block).
- New chats open on a `SovraSRX`-style welcome: a greeting, a plain-language subtitle and
  starter chips that fill the message box without sending.

- Clean up the title bar: it no longer shows the product name or a light/dark
  button. Appearance now lives in **Settings → Appearance**.
- Rework Settings for people new to AI: three plain-language sections (Appearance,
  AI connection, Advanced), step-by-step OpenRouter key instructions, a folded-away
  Advanced section for other services, and technical connection fields (service
  type, key header, wait time, organization/project IDs) hidden behind
  *Show technical options*. Connection, key and error messages use everyday wording.
- Simplify **My AI's** for people new to AI: three plain choices (Automatic,
  One AI for everything, Choose per task) with one-line explanations; effort per task
  (Quick / Balanced / Thorough) shows only when choosing per task, and the decision
  engine, routing tree and prompt tester sit behind *How automatic choice works*.
- Rewrite the **Models** library in everyday words: Online / On my computer / Other
  services tabs, "Made by", "Reads / Creates", "Not for chat" markers, "Memory size", "Can use
  tools", "Thinks step by step", plain prices and clearer Ollama download help.

### Fixed

- Sending in a conversation restored at startup no longer blanks earlier replies or
  shows one reply's text in place of another: new message ids skip ids already in
  the conversation, and chat rendering state is keyed by position, so duplicate ids
  saved by older builds still render correctly.
- Code blocks use the dark highlight theme in dark mode (they used the light one),
  and reasoning text is muted.
- Memory recall's "newest first" order no longer flips when two facts are saved in
  the same millisecond: each store hands out strictly increasing fact timestamps.
- Ensure every clickable button (`design::button`, `design::primary_button`), checkbox (`Checkbox`), dropdown (`Select`), and custom interactive element across `sovra-ui` displays a pointer cursor (`CursorStyle::PointingHand`), enforced by `xtask check-styles`.
- Normal chat answers conversationally in Markdown; native UI tools are optional,
  and ordinary turns no longer invoke the specialist routing simulation.
- Preserve streamed text exactly and deduplicate ADK's final response snapshots.
- Render selectable Markdown with code copying in a centered reading column;
  virtualized messages follow streamed output, pause when scrolled up, and offer
  Jump to latest. Enter sends, Shift+Enter inserts a line, and Stop clears queued work.

- Keep OpenRouter key management visible after saving, with a persistent masked
  saved status and explicit replace, test and remove controls.

- Enable keyboard navigation immediately when a window opens, before an input has been clicked.
- Size the title bar to fit the appearance control without clipping and verify keyboard theme switching.
- Align model names with their metadata and preserve usable Models controls at the minimum window size.
- Settings now opens provider configuration, with a masked OpenRouter key editor,
  connection testing and OS keychain save/replace/removal.
- Left sidebar icons and labels share a left-aligned content edge.
- Chat starts empty; removed seeded messages, hero banner and mock build inspector.
- ADK-UI tool responses now deliver actual AG-UI component payloads to native GPUI
  cards, tables, alerts, progress and action buttons. Surface revisions update in
  place; button actions carry their surface identity back to the agent.
### Security

- Record the approved `RUSTSEC-2026-0387` exception for SurrealDB's WebAssembly-only
  cryptography dependency, which is absent from all three desktop release targets.

### Added

- **Settings → Appearance** remembers Light or Dark across restarts (saved with model
  preferences; older preference files load as Dark).
- Show streamed reasoning in a collapsible panel with an elapsed-time indicator,
  and refresh chat typography, avatars, user bubbles and composer styling.
- Feed Markdown incrementally and save transcript snapshots at turn boundaries
  to reduce work during long streamed replies.
- Assistant replies show a compact model and connection pill with the exact model ID
  on hover. Execution identity stays with each reply across model changes and saved history.

- **Chat History Rail & Schedules Integration (`sovra-core`, `sovra-store-surreal`, `sovra-runtime`, `sovra-ui`)**:
  - Multi-session conversation history rail (`RecentConversationsRail` from `SovraSRXProduction`) with collapsible strip, instant search, segmented `Chats | Scheduled` radio-group filter pills, `Pinned` partitioning, recency buckets (`Today`, `Yesterday`, `Previous 7 Days`, `Previous 30 Days`, `Older`), semantic topic icons, auto-titling from first prompt, inline rename, pin/unpin, and soft-deletion.
  - Persistent `chat_thread` storage in `sovra-store-surreal` and automatic startup history hydration via `sovra-runtime`.
  - Schedules placeholder empty state (`"No scheduled runs yet"` in the history rail and `"No schedules yet"` with schedule creation form in the Schedules Command Center).
- **Main Chat, Login & Personal AI Builder (`sovra-ui`, `sovra-core`)**:
  - 3-column post-login workspace (`Sidebar`, `ChatView` with `"Built around you."` hero banner and prompt timeline counter, and `My Personal AI` right inspector with 6 configuration cards and `"Preview my AI"` workflow) plus a native glass Login screen.
  - Native AG-UI (`adk-ui = "2.2.0"`) surface rendering (`BuildProgress` live checklist card, `ClarifyChips`, and `SurfaceCard`) and `SovraSRX` type-ahead prompt queue strip.
  - `SovraSRX` **Schedules Command Center** (`NavTab::Schedules`) with KPI cards (`Active Schedules`, `Runs Today`, `30d Success Rate`), Pause/Resume controls, and one-click `"Run in Chat"` execution.
- **Multi-Provider Model Library & Discovery (`sovra-providers`, `sovra-adk`, `sovra-ui`)**:
  - Live catalog discovery, caching, and OS keychain credential storage (`keyring`) for **OpenRouter** (all models & modalities), **Ollama** (local tags, metadata enrichment, and streaming model pulls), **OpenAI-compatible** servers (`LM Studio`, `vLLM`, `llama.cpp`, custom `/v1` endpoints), **Anthropic**, and **Google Gemini**.
  - Virtualized **Models** view with search, capability/modality/context/price filters, side-by-side model comparison, and connection management.
- **22 `SovraSRX` Specialist Agents & `CogniRouteSLM` Dynamic Tree Router (`sovra-core`, `sovra-adk`, `sovra-ui`)**:
  - Full 22-specialist `SovraSRX` roster (`OrchestratorAgent`, `WebSearchAgent`, `CodingAgent`, `DeepResearchAgent`, `StockMarketAgent`, `HousingMarketAgent`, `ScholarAgent`, `GitHubAgent`, etc.) with per-agent model and tier (`Tier 0 · Fast`, `Tier 1 · Standard`, `Tier 2 · Deep`) overrides.
  - Three selectable model routing modes in **My AI's & Routing Tree**: `Single Global Model`, `Per-Agent Models (22 Specialists)`, and `Dynamic Tree Router`.
  - System-1 decision model client (`sovra_adk::decision_client`) supporting **Laya** (`/v1/systemone`), **OpenJev / TypeSafe Jev** (`/v1/systemone` and OpenRouter `/api/alpha/decisions`), and an **Embedded Rust Heuristic** engine (`0ms` offline fallback) with `heuristic_floor`, auditable node-by-node traces, and `rc1-<tree>-<path>` route codes.

## [0.1.0] — 2026-10-24

Initial import of the native Rust desktop harness.

### Added

- Workspace of layered crates with machine-enforced dependencies
  (`app → ui → runtime → core`, table in `xtask/src/deps.rs`):
  - `sovra-core` — pure domain: types, `Command`/`Event` protocol, state
    reducers, ports (`MemoryStore`). No IO, async runtime, or GUI.
  - `sovra-adk` — ADK agent harness adapter.
  - `sovra-providers` — LLM provider adapters.
  - `sovra-runtime` — tokio on a background thread, serving the UI over
    channels (`Client`).
  - `sovra-store-surreal` — embedded SurrealDB adapter for `MemoryStore`
    with bi-temporal, supersede-don't-edit facts.
  - `sovra-ui` — GPUI views via `gpui-kit`, with the native Sovra design
    system under `src/design/` (dark + light themes).
  - `sovra-app` — binary and composition root: Velopack hook, paths,
    logging, wiring.
- `xtask` automation: `dev` (watch + rebuild + relaunch), `run`, `ci`,
  `fix`, `check-deps` (layering), `check-styles` (styling contract and
  contrast), `setup`, `dist`, and the editor `hook` (rustfmt + clippy on
  every agent edit).
- CI (`.github/workflows/ci.yml`) running every gate, and release
  packaging (`.github/workflows/release.yml`) producing signed Velopack
  installers for macOS, Windows, and Linux from a `vX.Y.Z` tag.
- Dependency policy: `deny.toml` (license allowlist, no OpenSSL, no git
  deps), `clippy.toml`, workspace-level lints, pinned Rust 1.99.1.
- Documentation: `docs/architecture.md`, `docs/development.md`,
  `docs/styling.md`, agent rules in `AGENTS.md` (plus per-crate files).
