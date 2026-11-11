# Native development

How to build and launch SovraHarness locally: a one-shot `cargo xtask dev`,
no file-watcher reload, and how Development mode stores keys.

`cargo xtask dev` is a **manual, one-shot** debug launch. It builds and runs the
app once with `SOVRA_DEV=1`. There is **no file watcher and no auto-reload**.
After you edit Rust, manifests or assets, run `cargo xtask dev` again yourself
to pick up the changes. In-memory UI/runtime state resets on each launch.

The user-approved development mode sets `SOVRA_DEV=1`. Debug builds then keep
model keys in the OS app data directory under `dev-credentials/` (plaintext,
owner-only directory and files on Unix), avoiding repeated macOS keychain prompts
after rebuilds. This store is separate from release credentials and is never
included in packages or source control. Release builds always use the OS keychain,
even when that variable is set. No keys are copied between the two stores.

## One-time setup

```sh
cargo xtask setup   # pinned CI tools (cargo-deny, nextest, shear)
cargo xtask dev     # one-shot build + launch; re-run to reload
```

Install Rust with rustup; `rust-toolchain.toml` pins the workspace toolchain.
On Linux, install system packages once first with `./scripts/install-linux-deps.sh`.
On macOS, install the Xcode command-line build tools. On Windows, use the MSVC
Rust target with the Visual Studio C++ build tools and Windows SDK.
`just` is optional: it delegates to the same xtask commands.

For first-launch steps see [Getting started](getting-started.md); startup
variables and local paths are in [Configuration](configuration.md). Packaging
has additional prerequisites in [Releasing](releasing.md).

`cargo xtask setup` prefers cargo-binstall when available. Watchexec is **not**
part of the supported workflow and is not installed by setup.

The runner takes an OS-held exclusive lock at `target/dev/watch.lock` for the
life of that one-shot session. A stale file does not block a new run: the
operating system releases the lock when its owner exits. Do not remove the lock
file or run `just clear-cache` while a `dev` session is live.

## How a launch works

`xtask/src/dev.rs` owns the one-shot path:

- Reports the effective Cargo target size (from `cargo metadata`, so
  `CARGO_TARGET_DIR` is honoured) against the advisory budget (default 30 GiB,
  `SOVRA_TARGET_BUDGET_GIB=<n>` to change).
- Runs `cargo run --package sovra-app` with `SOVRA_DEV=1`.
- Defaults `RUST_BACKTRACE` to `1` when unset; keeps an existing value.
- Inherits `RUST_LOG`; without it, the app's focused debug filter applies.
- The xtask launcher removes its inherited Cargo package metadata before starting
  child tools so build-script fingerprints do not change merely because a build
  came through xtask; user Cargo configuration, target directories and logging
  variables remain intact.
- Child stdout/stderr are inherited directly. There is no `tee`/`tail` process.
  Daily rolling logs remain under the OS data directory.

There is **no** Watchexec (or other) supervisor, no save-triggered rebuild, and
no automatic relaunch when the app window closes. Closing the app ends the
session; start again with `cargo xtask dev` when you want another run.

## Monitoring the build cache

Cargo never garbage-collects a target directory: every lockfile bump, feature
change or flag change leaves a stale copy of the affected dependencies in
`target/debug/deps`. A long-lived target plus the per-edit clippy hook once grew
it to 85 GiB. Each `cargo xtask dev` reports growth without deleting outputs that
a running session or test command may still need:

- Above the warning threshold (default 30 GiB, `SOVRA_TARGET_BUDGET_GIB=<n>` to
  change) it reports the size and keeps building. This is an advisory threshold,
  not an enforced disk limit; `dev` never runs `cargo clean`.
- To reclaim space explicitly, stop any live `cargo xtask dev` session and finish
  other Cargo work, then run `just free-cache` (`just clear-cache` also works; same as
  `cargo xtask clean-cache reclaim`). That command reports the effective target
  size, refuses while `target/dev/watch.lock` is held, and runs profile-scoped
  `cargo clean` for reclaimable `dev` and `release` artefacts. Optional
  `--cargo-home` also deletes recreatable `~/.cargo` registry/src, registry/cache
  and git checkouts — never `bin`, config or credentials. Use
  `just clean-cache` (status) or `just free-cache --dry-run` to inspect without
  deleting. Do not wipe the cache after every build.
- Per-crate builds (the hook's `clippy -p <crate>`, per-crate tests) can resolve
  different dependency features than `cargo run` and so add duplicate dependency
  builds. Workspace feature unification would prevent that but is nightly-only in
  Cargo 1.99. The warning makes this growth visible without interrupting builds.
- Dependencies build without debug info in the dev profile; workspace crates keep
  line tables for backtraces.
- `build.jobs = 8` caps concurrent rustc/linker processes so a rebuild beside
  rust-analyzer and the running app does not exhaust RAM.
- On macOS, `target` may be a symlink to a `*.noindex` directory outside the
  checkout so Spotlight does not index build output.

The first build can take longer; later manual `dev` runs reuse the incremental
target directory. The full CI gates remain separate: do not run the entire test
suite on every save.

## Agent workflow

Read the shared root `AGENTS.md`.

- Edit sources as needed. Saving does **not** reload the app.
- When a reload is required, run `cargo xtask dev` (stop any previous session
  first if the lock is held).
- Run `cargo xtask ci` for the definition of done.
- Do not start Watchexec, `cargo watch`, or another auto-rebuild loop.
- Prefer `cargo xtask dev` over a bare `target/.../sovra` when you need the
  development credential store and Development-mode startup policy.

## Sign-in integration status

The sign-in backend and the desktop flow are wired end to end. On
`Event::Ready` the shell sends `AuthCommand::Load`; the restored session (or its
absence) routes to the workspace or the Login screen. The form sends
`AuthCommand::SignIn`, or `AuthCommand::CreateAccount` on a fresh install with
no accounts, and waits for `AuthEvent::SignedIn`; sign-out sends
`AuthCommand::SignOut`. The Google button sends `AuthCommand::StartGoogleSignIn`;
`AuthEvent::GoogleSignInStarted` opens the consent URL in the browser and the
Login screen shows a waiting panel with *Open the page again* and *Cancel*
(`AuthCommand::CancelGoogleSignIn`) until the loopback listener reports back.

Remaining rollout work, in honesty: a dedicated create-account mode (separate
name field and a live password-strength meter rather than server-side errors),
and a live end-to-end run of the Google flow against real client credentials —
the exchange and listener are covered by tests, not by a production Google app.

The implemented parts are:

- `sovra-core::auth`: account/session types, validation, commands/events and the
  reducer used to update the workspace route and displayed profile.
- `sovra-providers::auth`: Argon2id password hashing and verification, Google
  authorization-code exchange with PKCE, and user-info retrieval.
- `sovra-runtime::auth`: the account worker, loopback listener, browser-wait
  cancellation, credential-store access and orderly task shutdown.
- `sovra-store-surreal`: account records and the current session persisted as JSON
  in the existing `team:auth` record. Passwords are stored as PHC hashes; signing
  out clears the session without deleting accounts. Google refresh tokens use
  the OS keychain, or the existing debug-only `SOVRA_DEV=1` credential store.

The Google adapter uses the desktop client saved in Settings, falling back to
`SOVRA_GOOGLE_CLIENT_ID` and optional `SOVRA_GOOGLE_CLIENT_SECRET` read once at
startup, then build-time values. Without a client, Settings offers setup and
Google sign-in stays unavailable. Supply environment values outside tracked
files. The backend binds an
OS-assigned port on `127.0.0.1` for `/callback`, waits up to five minutes for a
browser response, and gives HTTP requests a 30-second timeout.

Tests cover domain parsing/reducers, password verification, PKCE URL construction,
account/session storage round-trips, runtime dispatch/shutdown and the UI's wait
for sign-in confirmation. These checks do not establish a completed browser sign-in
flow or account-based data isolation. Save failures are currently logged rather
than propagated as sign-in failures; this remains rollout work.

## Autonomous native app testing

Use the opt-in [Development MCP server](development-mcp.md) to attach Codex, Claude
Code, or another MCP client to the running app. Its Settings switch, explicit
top-bar indicator, and authenticated local transport are separate from the
`SOVRA_DEV` credential exception. Agent verification rules are in `AGENTS.md`.

## Related docs

- [`docs/development-mcp.md`](development-mcp.md) — Development mode MCP bridge
- [`AGENTS.md`](../AGENTS.md) — agent rules for the manual `dev` workflow
