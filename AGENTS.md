# SovraHarnessRust — agent rules

Canonical rules for every coding agent (Codex, Cursor, Claude Code, …). Nested `AGENTS.md`
files in each crate add crate-specific rules; the closest file wins. Reasoning and sources:
`docs/architecture.md`.

Native Rust desktop app (a local LLM agent harness). GPUI via `gpui-kit` for UI, tokio for
async, Velopack for install + auto-update. Rust 1.99.1 (pinned), edition 2024.

## Commands

| Task                    | Command                     |
| ----------------------- | --------------------------- |
| Persistent dev + logs   | `cargo xtask dev` (keep running in `sovra:sovraRust`; inspect before starting) |
| One-off app launch      | `cargo xtask run` (outside the persistent dev workflow) |
| Every gate (must pass)  | `cargo xtask ci`            |
| Auto-fix fmt + clippy   | `cargo xtask fix`           |
| Layering check only     | `cargo xtask check-deps`    |
| Styling check only      | `cargo xtask check-styles`  |
| One crate's tests       | `cargo nextest run -p <crate>` (or `cargo test -p <crate>`) |
| Install dev tools       | `cargo xtask setup`         |
| Release build + package | `cargo xtask dist`          |

**Definition of done:** `cargo xtask ci` passes. Never report a task complete without running it;
if it fails, say so and show the output.

## Commits, Gitmoji & Changelog rule (mandatory for all agents & contributors)

- **Gitmoji commits:** Every commit message must start with a relevant gitmoji (e.g. `✨ feat(routing): …`, `🐛 fix(ui): …`, `📝 docs: …`).
- **Mandatory `CHANGELOG.md` update:** Every commit that adds, changes, fixes, or removes functionality **must** update `CHANGELOG.md` under `## [Unreleased]` as part of the commit, and keep `README.md` in sync when user-facing capabilities or workflows change. Never push a commit to `main` without updating `CHANGELOG.md`.

## Persistent development session (all agents)

- The normal dev runner stays in the `sovra` tmux session, window **`sovraRust`**.
  `cargo xtask dev` watches source/assets and automatically rebuilds and relaunches
  the native app on save. This is restart-based live reload, not in-process Rust
  hot swapping; in-memory UI/runtime state resets. See `docs/development.md`.
- **Leave the watcher running.** Ordinary edits, checks and task completion do not
  require stopping/restarting it. Do not send Ctrl-C, kill the app, start another
  `cargo xtask run`, respawn the pane, or create a competing watcher just to see changes.
- Never replace this pane's watcher with a direct `target/.../sovra` binary, including
  a binary built in an agent-specific target directory. That disables automatic
  rebuilds and can leave the user viewing stale code. A running app alone is not
  evidence of live reload: confirm that `xtask dev` and Watchexec supervise it.
- Inspect existing status and logs first:
  `tmux list-panes -t sovra:sovraRust -F '#{pane_id} #{pane_current_command} #{pane_dead}'`
  and `tmux capture-pane -p -t sovra:sovraRust -S -100`.
  Build output, restart events and application tracing all stream in that pane.
- A lock file is not proof of a live watcher. Verify the pane/process tree. If the
  watcher is genuinely absent, start **one** `cargo xtask dev` in the existing idle
  repo pane; never interrupt unrelated work. An OS-held lock rejects duplicates.
- Keep `target/`, logs and generated artifacts out of the watch inputs. Do not
  clear Cargo's target directory while the watcher owns its lock. Build errors
  stay visible; fix and save, and the watcher retries without manual intervention.
- Only change/restart the supervisor itself when its setup requires it or the user
  explicitly requests it. No launch-on-login service or production auto-restart is
  implied by this development workflow.

## Architecture (machine-enforced)

```
crates/
  sovra-core      pure domain: types, protocol (Command/Event), state reducers, ports
                  (MemoryStore). No IO/async runtime/GUI; async port *traits* are fine.
  sovra-adk       ADK agent harness adapter.
  sovra-providers  HTTP catalogs, connection validation, OS credentials and model metadata.
  sovra-runtime   tokio on a background thread; serves the UI over channels (Client).
  sovra-store-surreal  SurrealDB adapter for MemoryStore. The only crate naming surrealdb.
  sovra-ui        GPUI views. The only crate allowed to use gpui-kit.
  sovra-app       binary + composition root: velopack hook, paths, logging, wiring. No logic.
xtask/            automation: ci, hooks, layering rules (xtask/src/deps.rs)
```

Dependencies point inward: `app → ui → runtime → core`. The allowed edges and which crate may
own `tokio` / `gpui-kit` / `velopack` live in `xtask/src/deps.rs` — that table is the
architecture. Changing it requires a stated reason in the same change.

- UI ↔ runtime communicate **only** via `sovra_core::Command` (UI → runtime) and
  `sovra_core::Event` (runtime → UI) over `sovra_runtime::Client`. No shared mutable state.
- Decide in `sovra-core` (pure, unit-tested); do IO in adapters; render in `sovra-ui`.
- Add a crate only for a distinct dependency set or a real boundary (e.g. `sovra-provider-*`,
  `sovra-store`, `sovra-secrets`, `sovra-mcp`), never for tidiness. Name it `sovra-<thing>`,
  directory = crate name, add it to `LAYERS`.

## Where new code goes

- **New capability end-to-end:** add `Command`/`Event` variants in `sovra-core` (exhaustive
  enums — the compiler lists every `match` to update), handle in `sovra-runtime`, fold state
  with a pure reducer in `sovra-core`, render in `sovra-ui`.
- **Open-ended plugin sets** (LLM providers, tools, MCP servers, storage backends): one trait
  in `sovra-core`, one impl per adapter, registered once in the composition root. Adding one =
  one struct + one `impl`. Never copy-paste a sibling implementation; extract the shared part
  first.
- **Memory:** domain types + `MemoryStore` port in `sovra-core/src/memory.rs`; SurrealDB
  specifics only in `sovra-store-surreal`. Pure retrieval logic (RRF fusion, validity) is in
  core where it's unit-tested. Facts are bi-temporal (`created/valid/invalid/expired_at`) and
  never deleted — supersede, don't edit. The backend and local-vs-remote choice live in one
  place: `sovra-app`.
- **Configuration / env / paths:** read once in `sovra-app`, pass values down.

## Rust rules

- No `unsafe` (forbidden). No `unwrap()` outside tests; use `?`, or `expect("why this holds")`
  only for true invariants.
- Errors: one `thiserror` enum per library crate boundary; `anyhow` only in `sovra-app` and
  `xtask`. Add context (`.context(..)`) where an error crosses a layer.
- Logging: `tracing` macros with structured fields (`tracing::info!(%id, "…")`). Never
  `println!`/`eprintln!`/`dbg!` (denied by lints).
- Async: only in `sovra-runtime`. Never block the UI thread; never `std::thread::sleep`.
  Spawn tasks through `sovra-runtime` so each has an owner and cancellation.
- Prefer exhaustive `match` over `_ =>` on our own enums. No `#[non_exhaustive]` on
  internal types.
- Keep items private by default; `pub(crate)` inside a crate; `pub` only for the crate's API.
- Every dependency is declared once in root `[workspace.dependencies]`; members use
  `{ workspace = true }`. Pin GPUI exactly (`=x.y.z`). Remove unused deps (cargo-shear fails CI).
- Comments explain *why*, not *what*. Every crate has a `//!` doc saying what it owns.

## Testing

- Pure logic: unit tests next to the code (`#[cfg(test)] mod tests`).
- Runtime: drive it through `Client` and assert on `Event`s.
- UI: headless GPUI tests with `#[gpui_kit::test]` and `Client::loopback()` to script events
  (see `crates/sovra-ui/tests/shell.rs`).
- Bug fix ⇒ regression test first.

## Never

- Loosen `[workspace.lints]`, `clippy.toml`, `deny.toml` or `xtask/src/deps.rs` to make a check
  pass. Fix the code. If a rule is genuinely wrong, say so and explain — don't silently change it.
- Add `#[allow(..)]` without `reason = "…"`; prefer `#[expect(..)]`.
- Add `tokio`, `gpui-kit`, or `velopack` to a crate other than its owner.
- Add OpenSSL (banned; use rustls), git dependencies, or wildcard versions.
- Commit or log secrets. Installed/release builds keep API keys in the OS keychain
  (keyring). The user-approved development exception is a debug build with
  `SOVRA_DEV=1`: keys may live in the app data directory's `dev-credentials/`,
  outside the checkout, with owner-only directory/file permissions on Unix.
  Release builds must ignore this flag and always use the keychain. Never copy
  development keys into the installer or migrate keychain entries automatically.

## GPUI notes

- `gpui-kit` changes weekly and has no semver promise. **Verify APIs against the pinned source**
  (`~/.cargo/registry/src/*/gpui-kit-0.8.0`, `gpui-component-0.8.0`, `gpui-pre-0.3.9`) or
  docs.rs for that exact version — not memory or older GPUI examples.
- Import GPUI via `gpui_kit::*` and components via `gpui_kit::component::…`; never depend on
  `gpui` directly.
- Views are `Entity<T>` + `impl Render`; keep `Task`/`Subscription` handles in the view
  (`_events: Task<()>`) or they are cancelled. Call `cx.notify()` after state changes.
- UI work must follow the shared styling contract below and `docs/styling.md`.

## Sovra styling (all agents)

- The canonical native system is `crates/sovra-ui/src/design/`, migrated from the
  current SovraSRXProduction product theme. Provenance, web research, native adaptations
  and examples: `docs/styling.md`. Read it before changing UI appearance.
- Use `cx.theme()` for GPUI colours; `design::tokens(cx)` for Sovra spacing, radii,
  materials, geometry and motion; `SovraStyled` for surface/text recipes. Raw colours
  and reusable visual constants belong only in `design/`. Name roles by purpose.
- Use GPUI controls for interaction/accessibility and `design::button` for button
  geometry. Keep visible keyboard focus, labels, disabled/hover/pressed states and
  readable text in **both** registered themes. Respect `cx.reduce_motion()`.
- Apply theme edits through `Theme::update` and switch through `Theme::change`.
  Never mutate `Theme::global_mut` directly or keep separate theme state in views.
- Branding: only `assets/branding/sovra-logo.svg`, embedded via `assets.rs`. Preserve
  its colours using `img`, not the monochrome `svg` mask; supply an accessible name.
  No raster variants, remote fonts/assets or runtime working-directory assumptions.
- Extend this system before inventing local styles. Do not copy the web stack or
  duplicate token values/rules for individual agents. `check-styles` runs in CI and
  hooks; keep the contrast and headless rendering checks passing with `cargo xtask ci`.
