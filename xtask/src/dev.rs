//! Persistent native development loop. Watchexec owns filesystem watching and child groups;
//! every rebuild reports shared Cargo target usage without deleting live build outputs.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

pub(crate) const WATCH_TOOL: &str = "watchexec-cli@2.7.2";

/// Default warning threshold for the whole target directory. A cold debug build plus the
/// hook's clippy/test artefacts is ~15 GiB; an unpruned watcher reached 85 GiB.
const DEFAULT_BUDGET_GIB: u64 = 30;
const BUDGET_ENV: &str = "SOVRA_TARGET_BUDGET_GIB";
const GIB: u64 = 1024 * 1024 * 1024;

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
    println!(
        "Target directory warning threshold: {} GiB (override with {BUDGET_ENV}).",
        budget_gib()
    );

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
    ]);
    // Through Cargo so edits to cycle diagnostics take effect on the next
    // watched change without replacing the running supervisor.
    command.args([
        env!("CARGO"),
        "run",
        "--quiet",
        "--package",
        "xtask",
        "--",
        "dev-cycle",
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

/// One watcher cycle: report disk usage, then build and launch the app.
pub(crate) fn cycle(root: &Path) -> anyhow::Result<()> {
    report_budget(&target_dir(root)?, budget_gib().saturating_mul(GIB));
    crate::run(
        Command::new(env!("CARGO"))
            .args(["run", "--package", "sovra-app"])
            .current_dir(root),
    )
}

fn budget_gib() -> u64 {
    std::env::var_os(BUDGET_ENV)
        .and_then(|value| value.to_str()?.trim().parse().ok())
        .filter(|&gib| gib > 0)
        .unwrap_or(DEFAULT_BUDGET_GIB)
}

/// Stale dependency variants (lockfile bumps, feature or flag changes) pile up in
/// `target/debug/deps` forever; Cargo has no stable target-directory GC. A build
/// lock does not protect binaries already running or about to be launched by
/// nextest, so watcher cycles must never clean the shared target directory.
fn report_budget(target: &Path, budget: u64) {
    let used = dir_size(target);
    if used <= budget {
        return;
    }
    println!(
        "Sovra dev: target directory is {:.1} GiB (warning threshold {} GiB).",
        gib(used),
        budget / GIB
    );
    println!(
        "Build outputs were kept because the watcher and other Cargo commands may need them. \
         To reclaim space, run `cargo clean --profile dev` only after the watcher and all \
         other Cargo work have stopped."
    );
}

/// The effective target directory, honouring `CARGO_TARGET_DIR` and Cargo config.
fn target_dir(root: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version=1", "--no-deps", "--offline"])
        .current_dir(root)
        .output()
        .context("run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse cargo metadata")?;
    metadata["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .context("cargo metadata has no target_directory")
}

/// Apparent size of every file below `path`, without following symlinks.
fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |meta| meta.len()),
            _ => 0,
        })
        .sum()
}

#[expect(
    clippy::cast_precision_loss,
    reason = "a one-decimal human-readable size"
)]
fn gib(bytes: u64) -> f64 {
    bytes as f64 / GIB as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = std::env::temp_dir().join(format!("sovra-xtask-size-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("deps/nested")).unwrap();
        std::fs::write(dir.join("a.rlib"), [0_u8; 100]).unwrap();
        std::fs::write(dir.join("deps/nested/b.rlib"), [0_u8; 23]).unwrap();
        let size = dir_size(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(size, 123);
    }

    #[test]
    fn missing_directory_has_no_size() {
        assert_eq!(dir_size(Path::new("/nonexistent/sovra/target")), 0);
    }

    #[test]
    fn exceeding_dev_budget_preserves_live_build_outputs() {
        let dir = std::env::temp_dir().join(format!("sovra-xtask-budget-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("debug/deps")).unwrap();
        let binary = dir.join("debug/deps/running-test");
        let contents = b"build output still needed by the watcher and CI";
        std::fs::write(&binary, contents).unwrap();

        report_budget(&dir, 1);

        assert_eq!(std::fs::read(&binary).unwrap(), contents);
        assert_eq!(dir_size(&dir), contents.len() as u64);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
