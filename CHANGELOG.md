# Changelog

All notable changes to SovraHarness are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow
[Semantic Versioning](https://semver.org/). Releases are cut by pushing a
`vX.Y.Z` tag (see [README → Shipping](README.md#shipping)); installed apps
auto-update from the resulting GitHub Release via Velopack.

Add entries under **Unreleased** as part of the change that introduces them;
move them under a version heading when the tag is cut.

## [Unreleased]

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
