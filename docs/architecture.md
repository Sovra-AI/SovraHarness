# Architecture and decisions

Why the repo looks the way it does. The binding, short version for agents is `AGENTS.md`.
Research date: 2026-10-23; versions verified on crates.io / official release notes.

## Priorities

1. **Ease of deployment and use** — one native binary, signed installers, silent auto-update,
   a polished accessible UI, and `cargo xtask run` as the only onboarding step.
2. **Scalable without repetition** — boundaries enforced by tools, one obvious place for each
   kind of code, extension points that make "add a provider/tool" one struct + one impl.
3. **Agent-proof** — the same rules for Codex, Cursor and Claude Code, enforced by the same
   checks locally (hooks) and in CI.

## Decisions

### UI: GPUI via `gpui-kit` (exact pin)

Pure-Rust candidates compared: GPUI + GPUI Kit, Slint 1.18, Iced 0.14, egui 0.37, Freya,
Dioxus Native, Xilem, Makepad.

- **GPUI Kit 0.8** ships what an LLM chat client needs out of the box: native Markdown with
  code highlighting, variable-height virtual lists (long transcripts), dock/tab panes, dialogs,
  forms, a Zed-grade text input, AccessKit accessibility and headless UI tests. Zed and
  Longbridge Pro prove the rendering and cross-platform distribution.
- Rejected: **Slint** (attribution required on the free licence; basic markdown, no code
  highlighting), **Iced** (no AccessKit yet), **egui** (tool-like look), **Freya** (single
  maintainer), **Dioxus Native / Xilem** (self-described not production-ready).
- Risk: GPUI has no semver promise and `gpui-kit` depends on a `gpui-pre` snapshot. Mitigation:
  exact pin (`=0.8.0`), GPUI confined to `sovra-ui`, deliberate bumps with `cargo xtask ci`.

### Native Sovra design system

The UI's Sovra theme and SVG migration is documented in [styling.md](styling.md).
`sovra-ui/src/design` owns typed semantic tokens and the GPUI theme adapter; assets
are embedded. `check-styles`, contrast checks and headless UI tests protect the
shared agent styling contract. No additional runtime dependency or crate is needed.

### Workspace: flat `crates/`, virtual manifest, hexagonal core

Following matklad's "Large Rust Workspaces" and the layout of Zed/uv: a virtual root manifest,
flat `crates/sovra-*`, every dependency/lint/package field inherited from the workspace, and
resolver 3 (MSRV-aware resolution).

- `sovra-core` is a functional core (no IO/async/GUI) so domain logic is deterministic and
  trivially testable; adapters (`runtime`, `ui`, later `provider-*`, `store`, `secrets`, `mcp`)
  surround it; `sovra-app` is the only composition root.
- Crates are split for distinct dependency sets or real boundaries (compile-time isolation:
  only `sovra-ui` rebuilds when GPUI changes), not for tidiness.
- Enforcement: `cargo xtask check-deps` (layering + external-crate ownership + lint
  inheritance, from `cargo metadata`), `cargo-deny` (advisories, licences, bans, sources,
  workspace-dependency hygiene), `cargo-shear` (unused deps), `clippy.toml` disallowed
  methods with reasons (shown to agents in lint output).

### UI ↔ runtime: message passing over runtime-agnostic channels

GPUI owns the main thread and has its own executor; tokio runs on a dedicated background
thread. They communicate only through `Command`/`Event` enums (defined in `sovra-core`) over
`async-channel`, which works on any executor. Consequences: the UI never blocks, never sees
tokio, and UI tests replace the runtime with `Client::loopback()`.

### Errors, logging

`thiserror` per library boundary, `anyhow` in the binary and xtask. `tracing` with a
non-blocking daily-rolling file appender (plus stderr in debug builds), filterable by `RUST_LOG`.

### Lints

`[workspace.lints]`: `unsafe_code = forbid`, clippy `pedantic` (priority −1 so specific
overrides win; noisy lints allowed, as in uv) plus restriction lints (`unwrap_used`,
`dbg_macro`, `todo`, `print_*`, `exit`). Tests may `unwrap`/`expect`.

### Task runner: `cargo xtask`

Pure Rust, nothing to install, cross-platform, and the single entry point used by humans, CI
and agent hooks — so a check can never pass locally and fail in CI for tooling reasons.

### Build profiles

Dev: `debug = "line-tables-only"` and dependencies at `opt-level = 2` (GPUI is slow
unoptimised; our crates stay fast to rebuild). Release: thin LTO, one codegen unit, stripped
debuginfo; unwinding kept so tool execution can be isolated with `catch_unwind` later.
`--profile profiling` for flamegraphs.

### Distribution: Velopack

One tool for installers and signed delta auto-update: `.pkg` (macOS, signed + notarized with
`codesign`/`notarytool`), `Setup.exe` (Windows, Azure Artifact Signing), AppImage (Linux).
`VelopackApp::build().run()` is the first line of `main`. Alternatives: `cargo-packager`
(slowing releases), `cargo-dist` (no GUI bundles), `self_update` (binary swap only).

Known issue: velopack 1.2.161 pins `zip ^8.6`, which has extraction advisories
(RUSTSEC-2026-0367/0368/0369). They are ignored in `deny.toml` with a reason (update packages
are first-party); remove the ignores when velopack moves to zip ≥ 9.

### Memory & persistence: SurrealDB behind a core port (decided 2026-10-23)

The agent's memory is a bi-temporal memory graph — the shape proven by Zep's Graphiti and
adopted over mem0-style flat memories:

- **Episodes** (raw user/agent/tool events) are immutable provenance, searched by full-text
  and recency — never embedded.
- **Facts** (distilled statements) carry an embedding and four temporal fields
  (`created_at`, `valid_at`, `invalid_at`, `expired_at`). Facts are superseded, never
  edited or deleted, so the agent can answer "what did I believe, and when".
- **Entities** and `mentions`/`derived_from`/`supersedes` RELATE edges form the graph.
- **Recall** is hybrid: HNSW vector leg + BM25 full-text leg, fused with reciprocal rank
  fusion. The fusion (`fuse_rrf`) is pure and lives in `sovra-core`, unit-tested.

**Why SurrealDB** (3.3.4, embedded): one engine covers documents, graph edges and HNSW
vector indexes in the same query — no Postgres+vector-DB+graph-DB stitching — and
`engine::any` makes deployment a connection string: `mem://` (tests), `surrealkv://dir`
(desktop default), `ws://host` (future remote server, an app-config change only). Pinned
≥ 3.3.4: earlier 3.x HNSW silently dropped near-duplicate vectors.

**Abstraction:** everything outside the adapter holds `Arc<dyn MemoryStore>` (the port in
`sovra-core`). `sovra-store-surreal` is the only crate naming SurrealDB (enforced by
check-deps); swapping backends = a new `sovra-store-*` crate + one line in `sovra-app`.
The runtime degrades gracefully: a store that fails to open or health-check logs an error
and the app runs without memory.

**Schema/migrations:** the ecosystem tools don't support SDK v3 embedded
(`surrealdb-migrations` pins v2; SurrealKit is beta), so the adapter applies its own
idempotent `DEFINE ... OVERWRITE` schema on every open (`IF NOT EXISTS` is a trap: it
keeps stale definitions), with `SCHEMA_VERSION` reserved for data migrations. SCHEMALESS
tables + explicit indexes while the shape is still evolving.

**Licence (business-relevant):** SurrealDB v3 is **BUSL-1.1** (→ Apache-2.0 on
2030-01-01). The grant permits this embedding — the app uses it for its own storage and
does not "provide database functionality to third parties". It is excepted per-crate in
`deny.toml`, not globally allowed. If Sovra ever exposes schema/table control to end
users, get legal review or a commercial licence first.

**Durability:** SurrealKV defaults to eventual durability (no fsync wait) and its on-disk
format is not frozen; back up via `db.export()` before upgrades (future `cargo xtask`
concern).

### Planned (create the crate when the first feature needs it)

- Secrets: `keyring` v4 (`keyring-core` + native stores) — API keys in the OS keychain.
- MCP: `rmcp`. Providers: one `sovra-provider-*` crate per API behind a `sovra-core` trait.

### Agent rules: one source of truth

- `AGENTS.md` (root + one per crate) is canonical. Codex and Cursor read it natively (nested
  files merge, closest wins).
- `CLAUDE.md` is `@AGENTS.md` plus Claude-only notes. Claude Code reads AGENTS.md natively only
  when no CLAUDE.md exists, so the import is the robust choice.
- Enforcement is by tooling, not prose: `cargo xtask hook` runs after edits via Claude Code's
  `PostToolUse` hook (`.claude/settings.json`, exit 2 feeds errors back) and Cursor's
  `afterFileEdit` hook (`.cursor/hooks.json`); `cargo xtask ci` is the definition of done for
  every agent and runs in CI.

## Sources

- Rust releases: https://blog.rust-lang.org/releases · 1.99.0: https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/
- GPUI Kit: https://github.com/longbridge/gpui-kit · GPUI AccessKit: https://github.com/zed-industries/zed/pull/56065
- Slint licence: https://slint.dev/pricing · Iced AccessKit PR: https://github.com/iced-rs/iced/pull/3111
- Large Rust workspaces: https://matklad.github.io/2021/08/22/large-rust-workspaces.html
- Cargo resolver 3: https://doc.rust-lang.org/cargo/reference/resolver.html
- cargo-deny: https://embarkstudios.github.io/cargo-deny/
- uv workspace lints: https://github.com/astral-sh/uv/blob/main/Cargo.toml · Zed: https://github.com/zed-industries/zed/blob/main/Cargo.toml
- Velopack: https://docs.velopack.io/getting-started/rust · signing: https://docs.velopack.io/packaging/signing
- keyring v4: https://docs.rs/crate/keyring/latest
- Claude Code memory/AGENTS.md: https://code.claude.com/docs/en/memory · hooks: https://code.claude.com/docs/en/hooks-guide
- Cursor rules: https://cursor.com/docs/context/rules · hooks: https://cursor.com/docs/agent/hooks
- Codex AGENTS.md: https://learn.chatgpt.com/docs/agent-configuration/agents-md · https://agents.md
- SurrealDB SDK: https://surrealdb.com/docs/sdk/rust · indexes: https://surrealdb.com/docs/surrealql/statements/define/indexes · BUSL grant: the LICENSE in the surrealdb 3.3.4 crate
- Graphiti bi-temporal model: https://github.com/getzep/graphiti · https://arxiv.org/abs/2501.13956
- mem0 memory ops: https://docs.mem0.ai/core-concepts/memory-operations
