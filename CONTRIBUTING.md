# Contributing to SovraHarness

This repository is **closed source and proprietary to Sovra** (see
[LICENSE](LICENSE)). Contributions come from Sovra personnel and authorized
collaborators only. Do not share code, discussions, or artifacts outside
Sovra.

The canonical engineering rules live in [AGENTS.md](AGENTS.md) (plus a short
`AGENTS.md` per crate) and apply to humans and coding agents alike. This file
covers the workflow around them; it does not duplicate them.

## Getting started

```sh
# Rust via https://rustup.rs — the pinned toolchain (1.99.1) installs on first build
# Linux only: ./scripts/install-linux-deps.sh
cargo xtask setup   # once: install pinned development tools
cargo xtask dev     # persistent watch + rebuild + relaunch (one per machine)
```

Read before writing code:

- [docs/architecture.md](docs/architecture.md) — why the layering is what it is
- [docs/development.md](docs/development.md) — the persistent dev session
- [docs/styling.md](docs/styling.md) — the native design system (required for UI work)

## Best practices

- **Definition of done:** `cargo xtask ci` passes locally. CI runs the same
  gates; don't lean on it to find what you could have caught first.
- **Decide in core, IO in adapters, render in UI.** New capabilities flow
  end-to-end: `Command`/`Event` variants in `sovra-core`, handling in
  `sovra-runtime`, a pure reducer in `sovra-core`, rendering in `sovra-ui`.
- **Fix the code, never the rules.** `[workspace.lints]`, `clippy.toml`,
  `deny.toml`, and `xtask/src/deps.rs` are not to be loosened to make a
  check pass. If a rule is genuinely wrong, raise it with reasoning.
- **Bug fix ⇒ regression test first.** Pure logic gets unit tests next to
  the code; runtime behavior is driven through `Client` and asserted on
  `Event`s; UI uses headless GPUI tests with `Client::loopback()`.
- **Dependencies** are declared once in root `[workspace.dependencies]`.
  No OpenSSL (rustls only), no git dependencies, no wildcard versions;
  remove unused deps (cargo-shear fails CI).
- **Secrets never land in the repo.** API keys go in the OS keychain
  (keyring), never in files, logs, or commit history. See
  [SECURITY.md](SECURITY.md).

## Branches, commits, and review

- Branch from `main`; keep branches short-lived and focused.
- Prefer small, reviewable commits. Prefix every commit message with a gitmoji and conventional scope (`✨ feat(ui): …`, `🐛 fix(runtime): …`, `📝 docs: …`). No attribution or co-author trailers.
- **Mandatory Changelog & README rule:** Every commit must update [CHANGELOG.md](CHANGELOG.md) under **Unreleased** before pushing to `main`, and update [README.md](README.md) whenever user-visible features, tabs, or commands change.
- Open a PR against `main`; CI must be green before merge.

## Releasing

Bump `version` in the root `Cargo.toml`, move the **Unreleased** changelog
entries under the new version, and push a `vX.Y.Z` tag.
`.github/workflows/release.yml` builds, signs, packages (Velopack), and
publishes the release that installed apps auto-update from.
