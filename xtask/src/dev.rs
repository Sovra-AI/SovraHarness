//! Persistent native development loop. Watchexec owns filesystem watching and child groups.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, bail};

pub(crate) const WATCH_TOOL: &str = "watchexec-cli@2.7.2";

pub(crate) fn run(root: &Path) -> anyhow::Result<()> {
    let state = root.join("target/dev");
    std::fs::create_dir_all(&state).context("create dev runner state directory")?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(state.join("watch.lock"))
        .context("open dev runner lock")?;
    lock.try_lock().context(
        "a dev watcher already owns this checkout; use the existing sovraRust tmux pane",
    )?;

    let version = Command::new("watchexec")
        .arg("-V") // The long form includes build metadata, not just the version.
        .output()
        .context("Watchexec is required; run `cargo xtask setup`")?;
    let expected = WATCH_TOOL
        .split_once('@')
        .expect("tool has a pinned version")
        .1;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != format!("watchexec {expected}")
    {
        bail!("install the verified {WATCH_TOOL} with `cargo xtask setup`");
    }

    println!("Sovra dev: watching Rust, manifests, Cargo config and embedded assets.");
    println!("Save to rebuild/relaunch. In-memory state resets; the watcher stays running.");
    println!("Compiler output and app tracing stream here. Leave this pane running.");

    let mut command = Command::new("watchexec");
    command.current_dir(root).args([
        "--restart",
        "--debounce=750ms",
        "--stop-signal=TERM",
        "--stop-timeout=5s",
        "--shell=none",
        "--no-meta",
        "--no-global-ignore",
        "--watch=crates",
        "--watch=Cargo.toml",
        "--watch=Cargo.lock",
        "--watch=rust-toolchain.toml",
        "--watch=.cargo",
        "--ignore=**/target/**",
        "--ignore=**/*.log",
        "--ignore=**/*.md",
        "--timings",
        "--print-events",
        "--",
        "cargo",
        "run",
        "--package",
        "sovra-app",
    ]);
    // Keep user-selected filters/backtraces. The application's debug logger
    // already writes to stderr and a rolling file; no tee/tail duplication.
    // Dev rebuilds change the binary signature; skip the keychain so reloads never prompt.
    command.env("SOVRA_DEV", "1");
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        command.env("RUST_BACKTRACE", "1");
    }
    let result = crate::run(&mut command);
    drop(lock); // Held across every rebuild; the OS releases it even after a crash.
    result
}
