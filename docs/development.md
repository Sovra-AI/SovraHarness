# Persistent native development

The shared development session is tmux **`sovra:sovraRust`**. Its pane runs
`cargo xtask dev` continuously and displays watcher events, Cargo build diagnostics,
panics/backtraces and application tracing. Switch to it with:

```sh
tmux select-window -t sovra:sovraRust
```

Save a Rust source, manifest, Cargo configuration or embedded asset file and the
runner rebuilds and relaunches Sovra. The window briefly closes during a rebuild;
in-memory state resets. A compile error leaves the diagnostics visible and the
watcher waiting for the next save. The watcher also stays alive if the app closes.
This is **restart-based live reload**, not in-process code replacement or preservation
of active agent work. Use test credentials/data for development.

The user-approved development mode sets `SOVRA_DEV=1`. Debug builds then keep
model keys in the OS app data directory under `dev-credentials/` (plaintext,
owner-only directory and files on Unix), avoiding repeated macOS keychain prompts
after rebuilds. This store is separate from release credentials and is never
included in packages or source control. Release builds always use the OS keychain,
even when that variable is set. No keys are copied between the two stores.

## One-time setup

```sh
cargo xtask setup   # pinned CI tools + watchexec-cli 2.7.2
cargo xtask dev     # run inside the existing idle sovraRust pane, once
```

Watchexec is a development executable, not a shipped dependency. It is pinned in
`xtask/src/dev.rs`. The runner checks its version and takes an OS-held exclusive
lock at `target/dev/watch.lock` before launching. A stale file does not block a new
runner: the operating system releases the lock when its owner exits. Do not remove
the lock file or run `cargo clean` under a live watcher.

The setup command prefers cargo-binstall when available. If a downloaded executable
does not run on a machine, compile the same pinned tool locally:

```sh
cargo install --locked watchexec-cli --version 2.7.2 --force
```

tmux keeps the runner alive across terminal detach/reattach, not computer reboots
or an explicitly terminated tmux server. After those events, check the pane and
start one watcher if absent. The repo does not install an OS login daemon.

## How it is supervised

`xtask/src/dev.rs` is the single source of watcher configuration:

- Native filesystem events with a 750ms debounce coalesce editor/agent save bursts.
- Watch `crates/`, root `Cargo.toml`/`Cargo.lock`, `.cargo/` and `rust-toolchain.toml`.
  Embedded SVGs are compiled into the binary, so asset edits rebuild too. New
  crates under `crates/` are automatically covered.
- Ignore metadata-only events, Markdown, logs and nested build output; respect
  project ignores but disable machine-specific global ignores. Build/test output
  under `target/`, agent rules and rolling logs cannot cause rebuild loops.
- Watchexec supervises the command's process session on macOS, process group on
  other Unix systems, or job object on Windows, so
  replacing Cargo also stops its native app child. On Unix it sends TERM, waits up
  to five seconds, then force-stops a child that has not exited. This bounds restart
  time; it does not promise application-level shutdown hooks on termination.
- `cargo run --package sovra-app` uses the project's incremental dev profile. No
  shell command interpolation, dynamic Rust libraries or alternate GUI stack.
- The xtask command launcher removes its inherited Cargo package metadata before
  starting child tools. This prevents build-script fingerprints from changing
  merely because a build came through xtask; user Cargo configuration, target
  directories and logging variables remain intact.
- Child stdout/stderr are inherited directly. There is no `tee`/`tail` process to
  duplicate log lines or become detached. tmux retains scrollback, and the existing
  daily rolling log remains available under the OS data directory.
- `RUST_BACKTRACE` defaults to `1`; an existing value is kept. `RUST_LOG` is inherited;
  without it, the app's existing focused debug filter applies. Avoid globally
  enabling trace logs for HTTP clients or logging keys/prompts.

## Monitoring the build cache

Cargo never garbage-collects a target directory: every lockfile bump, feature
change or flag change leaves a stale copy of the affected dependencies in
`target/debug/deps`. A long-lived watcher plus the per-edit clippy hook once grew
it to 85 GiB. The dev loop reports growth without deleting outputs that a running
app, watcher or test command may still need:

- Each watcher cycle runs `cargo xtask dev-cycle`, which measures the effective
  target directory (from `cargo metadata`, so `CARGO_TARGET_DIR` is honoured).
  Above the warning threshold (default 30 GiB, `SOVRA_TARGET_BUDGET_GIB=<n>` to
  change) it reports the size and keeps building. This is an advisory threshold,
  not an enforced disk limit; the watcher never runs `cargo clean`.
- To reclaim space explicitly, first stop the watcher and finish all other Cargo
  work, then run `cargo clean --profile dev`. This removes dev-profile outputs and
  requires a cold rebuild; release/dist output is left alone. Cleaning during live
  development can delete test binaries between compilation and execution, even
  when Cargo's build lock is respected. Ordinary edits and checks must leave the
  watcher running; schedule cleanup separately when development is stopped.
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

The first build can take longer; subsequent saves reuse the incremental target
directory. Edits during a build cancel that build and schedule the newest state.
The full CI gates remain separate: do not run the entire test suite on every save.

## Agent workflow

Read the shared root `AGENTS.md`. Before starting anything, inspect:

```sh
tmux list-panes -t sovra:sovraRust -F '#{pane_id} #{pane_current_command} #{pane_dead}'
tmux capture-pane -p -t sovra:sovraRust -S -100
```

Then edit, inspect the automatic rebuild/logs, and run `cargo xtask ci` for the
definition of done. **Leave the dev runner active when the task ends.** Do not
launch extra app instances, clear the pane or send Ctrl-C to obtain a fresh log.
Never replace the runner with a directly launched `target/.../sovra` binary,
including one from an agent-specific build directory: that app will not reload.
Use tmux scrollback/capture to inspect it. A source error is fixed by another save,
not by restarting the watcher. A change to the watcher configuration itself is a
supervisor change and may require a deliberate one-time restart.

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

The app reads `SOVRA_GOOGLE_CLIENT_ID` once at startup. An absent or blank ID
disables the Google backend. `SOVRA_GOOGLE_CLIENT_SECRET` is optional. Supply these
through the development environment, not tracked files. The backend binds an
OS-assigned port on `127.0.0.1` for `/callback`, waits up to five minutes for a
browser response, and gives HTTP requests a 30-second timeout.

Tests cover domain parsing/reducers, password verification, PKCE URL construction,
account/session storage round-trips, runtime dispatch/shutdown and the UI's wait
for sign-in confirmation. These checks do not establish a completed browser sign-in
flow or account-based data isolation. Save failures are currently logged rather
than propagated as sign-in failures; this remains rollout work.

## Research and rationale

Checked online on 2026-10-23 and against the pinned CLI source:

- [Watchexec upstream](https://github.com/watchexec/watchexec): native file watching,
  debounce and process-group supervision across supported desktop platforms.
- [Watchexec CLI manual](https://github.com/watchexec/watchexec/blob/main/doc/watchexec.1.md):
  restart policy, watch/ignore filters, stop signal/timeout and inherited command I/O.
- [Pinned 2.7.2 release](https://github.com/watchexec/watchexec/releases/tag/v2.7.2).
- [cargo-watch upstream](https://github.com/watchexec/cargo-watch): archived; use
  the maintained Watchexec CLI for a new setup.
- [GPUI Shell](https://github.com/longbridge/gpui-kit/blob/main/crates/shell/README.md):
  its in-process reload facility applies to script-based applications. This repo
  renders native Rust views, so we retain the existing architecture and rebuild.
- [tmux guide](https://github.com/tmux/tmux/wiki/Getting-Started): sessions survive
  terminal detachment and can be reattached.
- [Cargo environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html):
  Cargo exports package metadata when running binaries; child Cargo processes
  must compute their own metadata rather than inherit xtask's package identity.

This repository's restart-based approach is an implementation decision based on
the current Rust/GPUI architecture, not a claim that GPUI requires Watchexec.

## Autonomous native app testing

Use the opt-in [Development MCP server](development-mcp.md) to attach Codex, Claude
Code, or another MCP client to the running app. Its Settings switch, explicit
top-bar indicator, and authenticated local transport are separate from the
`SOVRA_DEV` credential exception. Agent verification rules are in `AGENTS.md`.
