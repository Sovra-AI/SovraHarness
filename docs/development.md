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
