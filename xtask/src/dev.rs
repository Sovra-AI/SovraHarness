//! Manual native development launch. One build + run per `cargo xtask dev`;
//! there is no file watcher or auto-reload — re-run `dev` yourself to pick up edits.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

/// Default warning threshold for the whole target directory. A cold debug build plus the
/// hook's clippy/test artefacts is ~15 GiB; an unpruned long-lived target once reached 85 GiB.
pub(crate) const DEFAULT_BUDGET_GIB: u64 = 30;
pub(crate) const BUDGET_ENV: &str = "SOVRA_TARGET_BUDGET_GIB";
pub(crate) const GIB: u64 = 1024 * 1024 * 1024;

/// Exclusive OS lock held by `cargo xtask dev` while that one-shot session runs.
#[derive(Debug)]
pub(crate) struct WatchLock {
    _file: File,
}

impl WatchLock {
    /// Lock file under the effective Cargo target directory (`…/dev/watch.lock`).
    pub(crate) fn path(root: &Path) -> PathBuf {
        watch_state_dir(root).join("watch.lock")
    }
}

/// `target/dev` (or `$CARGO_TARGET_DIR/dev`), where the one-shot dev session keeps its lock.
pub(crate) fn watch_state_dir(root: &Path) -> PathBuf {
    // Fall back to `<root>/target` when metadata is unavailable (unit-test roots).
    target_dir(root)
        .unwrap_or_else(|_| root.join("target"))
        .join("dev")
}

/// Acquire the dev-session lock, or fail if another `xtask dev` already holds it.
/// Callers that reclaim `target/` must hold this across the clean.
pub(crate) fn try_watch_lock(root: &Path) -> anyhow::Result<WatchLock> {
    let lock_path = WatchLock::path(root);
    let state = lock_path
        .parent()
        .context("watch.lock path missing parent directory")?;
    std::fs::create_dir_all(state).context("create dev runner state directory")?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .context("open dev runner lock")?;
    match file.try_lock() {
        Ok(()) => Ok(WatchLock { _file: file }),
        Err(TryLockError::WouldBlock) => {
            bail!("a live `cargo xtask dev` session holds target/dev/watch.lock")
        }
        Err(TryLockError::Error(error)) => Err(error).context("lock dev runner lock"),
    }
}

/// One-shot: report disk usage, then build and launch the app with `SOVRA_DEV=1`.
/// No file watching — re-run `cargo xtask dev` to reload after edits.
pub(crate) fn run(root: &Path) -> anyhow::Result<()> {
    let lock = try_watch_lock(root).context(
        "another `cargo xtask dev` already owns this checkout; stop it before starting again",
    )?;

    println!("Sovra dev: one-shot build and launch (no auto-reload).");
    println!("Re-run `cargo xtask dev` yourself after edits to pick up changes.");
    println!(
        "Target directory warning threshold: {} GiB (override with {BUDGET_ENV}).",
        budget_gib()
    );

    report_budget(&target_dir(root)?, budget_bytes());

    let mut command = Command::new(env!("CARGO"));
    command
        .args(["run", "--package", "sovra-app"])
        .current_dir(root);
    // Dev rebuilds change the binary signature; skip the keychain so reloads never prompt.
    command.env("SOVRA_DEV", "1");
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        command.env("RUST_BACKTRACE", "1");
    }
    let result = crate::run(&mut command);
    drop(lock); // Held for this session; the OS releases it even after a crash.
    result
}

pub(crate) fn budget_gib() -> u64 {
    std::env::var_os(BUDGET_ENV)
        .and_then(|value| value.to_str()?.trim().parse().ok())
        .filter(|&gib| gib > 0)
        .unwrap_or(DEFAULT_BUDGET_GIB)
}

pub(crate) fn budget_bytes() -> u64 {
    budget_gib().saturating_mul(GIB)
}

/// Stale dependency variants (lockfile bumps, feature or flag changes) pile up in
/// `target/debug/deps` forever; Cargo has no stable target-directory GC. A build
/// lock does not protect binaries already running or about to be launched by
/// nextest, so a live `cargo xtask dev` session must never clean the shared target.
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
        "Build outputs were kept because this session and other Cargo commands may need them. \
         To reclaim space when idle, stop `cargo xtask dev` and run \
         `just clear-cache`."
    );
}

/// The effective target directory, honouring `CARGO_TARGET_DIR` and Cargo config.
pub(crate) fn target_dir(root: &Path) -> anyhow::Result<PathBuf> {
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
pub(crate) fn dir_size(path: &Path) -> u64 {
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
pub(crate) fn gib(bytes: u64) -> f64 {
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
