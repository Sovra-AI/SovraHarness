//! Explicit reclaim of Cargo build caches when no `cargo xtask dev` session is live.
//!
//! Leaves a running `cargo xtask dev` alone: deletion is gated on `target/dev/watch.lock`.
//! Prefer this over hand-written `rm -rf target` so agents and humans share one
//! safety gate and a measured budget (`SOVRA_TARGET_BUDGET_GIB`, default 30 GiB).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

use crate::dev::{
    BUDGET_ENV, DEFAULT_BUDGET_GIB, budget_bytes, budget_gib, dir_size, gib, target_dir,
    try_watch_lock,
};

/// Recreatable Cargo-home subtrees. Never includes `bin`, config or credentials.
const CARGO_HOME_CACHE_DIRS: &[&str] =
    &["registry/cache", "registry/src", "git/db", "git/checkouts"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Status,
    Reclaim { dry_run: bool, cargo_home: bool },
}

pub(crate) fn run(root: &Path, args: &[String]) -> anyhow::Result<()> {
    let mode = parse_args(args)?;
    match mode {
        Mode::Status => status(root),
        Mode::Reclaim {
            dry_run,
            cargo_home,
        } => reclaim(root, dry_run, cargo_home),
    }
}

fn parse_args(args: &[String]) -> anyhow::Result<Mode> {
    let (verb, flags) = match args.first().map(String::as_str) {
        None => ("status", args),
        Some("status") => ("status", &args[1..]),
        Some("reclaim") => ("reclaim", &args[1..]),
        Some(other) if other.starts_with('-') => ("status", args),
        Some(other) => bail!("unknown clean-cache command `{other}`\n\n{HELP}"),
    };
    let mut dry_run = false;
    let mut cargo_home = false;
    for arg in flags {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--cargo-home" => cargo_home = true,
            "-h" | "--help" => bail!("{HELP}"),
            other => bail!("unknown clean-cache argument `{other}`\n\n{HELP}"),
        }
    }
    match verb {
        "status" => {
            if dry_run || cargo_home {
                bail!("`status` does not take --dry-run or --cargo-home\n\n{HELP}");
            }
            Ok(Mode::Status)
        }
        "reclaim" => Ok(Mode::Reclaim {
            dry_run,
            cargo_home,
        }),
        other => bail!("unknown clean-cache command `{other}`\n\n{HELP}"),
    }
}

const HELP: &str = "\
cargo xtask clean-cache [status]
cargo xtask clean-cache reclaim [--dry-run] [--cargo-home]
just clear-cache [--dry-run] [--cargo-home]

  status     Report the effective Cargo target size against the budget
             (default 30 GiB / SOVRA_TARGET_BUDGET_GIB). No deletion.
  reclaim    Remove reclaimable workspace build artifacts (cargo clean
             --profile dev, then --release). Explicit reclaim always cleans
             those profiles. Refuses while cargo xtask dev holds
             target/dev/watch.lock. Prefer `just clear-cache` for the same
             path.
  --dry-run  Print the reclaim plan without deleting anything.
  --cargo-home
             Also delete recreatable ~/.cargo registry/src, registry/cache
             and git checkouts. Never touches bin, config or credentials.

There is no auto-reload watcher. Reclaim only when no `cargo xtask dev` session
is running; do not wipe the cache after every build.
";

fn status(root: &Path) -> anyhow::Result<()> {
    let target = target_dir(root)?;
    let used = dir_size(&target);
    let budget = budget_bytes();
    println!(
        "Sovra clean-cache: target {} is {:.1} GiB (budget {} GiB; override with {BUDGET_ENV}).",
        target.display(),
        gib(used),
        budget_gib()
    );
    if used > budget {
        println!(
            "Over budget by {:.1} GiB. When no `cargo xtask dev` session is \
             running, run `just clear-cache`.",
            gib(used - budget)
        );
    } else {
        println!(
            "Under budget (warning threshold {DEFAULT_BUDGET_GIB} GiB). To free \
             reclaimable build artefacts when idle, run `just clear-cache`."
        );
    }
    if let Some(home) = cargo_home_dir() {
        let home_used: u64 = CARGO_HOME_CACHE_DIRS
            .iter()
            .map(|rel| dir_size(&home.join(rel)))
            .sum();
        println!(
            "Optional Cargo-home caches ({}/{{registry/{{cache,src}},git/{{db,checkouts}}}}): \
             {:.1} GiB. Reclaim with `just clear-cache --cargo-home`.",
            home.display(),
            gib(home_used)
        );
    }
    Ok(())
}

fn reclaim(root: &Path, dry_run: bool, cargo_home: bool) -> anyhow::Result<()> {
    // Report size before the lock gate so a refused reclaim still tells the user
    // how much disk the build tree is using.
    let target = target_dir(root)?;
    let before = dir_size(&target);
    let budget = budget_bytes();
    println!(
        "Sovra clean-cache: target {} is {:.1} GiB before reclaim (budget {} GiB).",
        target.display(),
        gib(before),
        budget_gib()
    );

    let steps = reclaim_steps(cargo_home);

    // Dry-run prints the plan without needing the lock, then still checks the
    // gate so callers learn whether a real reclaim would be refused right now.
    if dry_run {
        for step in &steps {
            match step {
                ReclaimStep::CleanProfile { profile } => {
                    println!("dry-run: would run `cargo clean --profile {profile}`");
                }
                ReclaimStep::PruneCargoHome => {
                    let home = cargo_home_dir().context("CARGO_HOME / ~/.cargo is unavailable")?;
                    for rel in CARGO_HOME_CACHE_DIRS {
                        let path = home.join(rel);
                        if path.exists() {
                            println!(
                                "dry-run: would remove recreatable {} ({:.1} GiB)",
                                path.display(),
                                gib(dir_size(&path))
                            );
                        }
                    }
                }
            }
        }
        println!(
            "Sovra clean-cache: target is {:.1} GiB after reclaim (budget {} GiB).",
            gib(before),
            budget_gib()
        );
        try_watch_lock(root).context(
            "refusing to reclaim while `cargo xtask dev` owns target/. Stop that \
             session first, then reclaim",
        )?;
        return Ok(());
    }

    // Hold the watcher lock for the whole reclaim so a concurrent `xtask dev`
    // cannot start mid-clean, and so we refuse while one is already live.
    let lock = try_watch_lock(root).context(
        "refusing to reclaim while `cargo xtask dev` owns target/. Stop that \
         session first, then reclaim",
    )?;

    for step in &steps {
        match step {
            ReclaimStep::CleanProfile { profile } => {
                println!("Running `cargo clean --profile {profile}`…");
                crate::run(
                    Command::new(env!("CARGO"))
                        .args(["clean", "--profile", profile])
                        .current_dir(root),
                )?;
            }
            ReclaimStep::PruneCargoHome => {
                let home = cargo_home_dir().context("CARGO_HOME / ~/.cargo is unavailable")?;
                prune_cargo_home_caches(&home)?;
            }
        }
    }

    let after = dir_size(&target);
    println!(
        "Sovra clean-cache: target is {:.1} GiB after reclaim (budget {} GiB).",
        gib(after),
        budget_gib()
    );
    if after > budget {
        println!(
            "Still {:.1} GiB over budget after reclaimable profiles were cleaned. \
             Remaining size is typically release/dist output under target/; remove \
             those explicitly if you no longer need the installers.",
            gib(after - budget)
        );
    }
    drop(lock); // Held across every delete so a watcher cannot start mid-clean.
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReclaimStep {
    CleanProfile { profile: &'static str },
    PruneCargoHome,
}

/// Decide which reclaim steps to run. Pure so unit tests can drive it without Cargo.
///
/// Explicit reclaim always cleans reclaimable profiles: the user asked to clear
/// the build cache. Optional `--cargo-home` adds recreatable registry/git dirs.
fn reclaim_steps(cargo_home: bool) -> Vec<ReclaimStep> {
    let mut steps = vec![
        ReclaimStep::CleanProfile { profile: "dev" },
        ReclaimStep::CleanProfile { profile: "release" },
    ];
    if cargo_home {
        steps.push(ReclaimStep::PruneCargoHome);
    }
    steps
}

fn cargo_home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo"))
}

fn prune_cargo_home_caches(home: &Path) -> anyhow::Result<()> {
    // Refuse to touch anything outside the allowlist, including bin/ and config.
    for rel in CARGO_HOME_CACHE_DIRS {
        let path = home.join(rel);
        if !path.exists() {
            continue;
        }
        let before = dir_size(&path);
        println!(
            "Removing recreatable {} ({:.1} GiB)…",
            path.display(),
            gib(before)
        );
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("remove recreatable Cargo cache {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;

    use crate::dev::{GIB, WatchLock};

    #[test]
    fn parse_status_and_reclaim_flags() {
        assert_eq!(parse_args(&[]).unwrap(), Mode::Status);
        assert_eq!(parse_args(&["status".into()]).unwrap(), Mode::Status);
        assert_eq!(
            parse_args(&["reclaim".into()]).unwrap(),
            Mode::Reclaim {
                dry_run: false,
                cargo_home: false
            }
        );
        assert_eq!(
            parse_args(&["reclaim".into(), "--dry-run".into(), "--cargo-home".into()]).unwrap(),
            Mode::Reclaim {
                dry_run: true,
                cargo_home: true
            }
        );
        assert!(parse_args(&["status".into(), "--dry-run".into()]).is_err());
        assert!(parse_args(&["reclaim".into(), "--force".into()]).is_err());
    }

    #[test]
    fn reclaim_steps_always_clean_profiles_when_asked() {
        let profiles = reclaim_steps(false);
        assert_eq!(
            profiles,
            vec![
                ReclaimStep::CleanProfile { profile: "dev" },
                ReclaimStep::CleanProfile { profile: "release" },
            ]
        );
        let with_home = reclaim_steps(true);
        assert_eq!(
            with_home,
            vec![
                ReclaimStep::CleanProfile { profile: "dev" },
                ReclaimStep::CleanProfile { profile: "release" },
                ReclaimStep::PruneCargoHome,
            ]
        );
    }

    #[test]
    fn held_watch_lock_refuses_reclaim_and_deletes_nothing() {
        let root =
            std::env::temp_dir().join(format!("sovra-xtask-clean-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target/dev")).unwrap();
        let marker = root.join("target/dev/keep-me");
        std::fs::write(&marker, b"live watcher needs this").unwrap();

        let holder = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(WatchLock::path(&root))
            .unwrap();
        holder.try_lock().unwrap();

        let err = try_watch_lock(&root)
            .context(
                "refusing to reclaim while `cargo xtask dev` owns target/. Stop that \
                 session first, then reclaim",
            )
            .expect_err("held lock must refuse reclaim");
        let message = format!("{err:#}");
        assert!(
            message.contains("refusing to reclaim") && message.contains("watch.lock"),
            "refusal must name the dev-session lock: {message}"
        );
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            b"live watcher needs this",
            "refused reclaim must not delete target contents"
        );

        drop(holder);
        let acquired = try_watch_lock(&root).expect("unlocked watch.lock must be acquirable");
        drop(acquired);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unlocked_watch_lock_allows_dry_run_plan() {
        let root =
            std::env::temp_dir().join(format!("sovra-xtask-clean-free-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target/dev")).unwrap();
        let lock = try_watch_lock(&root).expect("idle checkout can take the watch lock");
        let steps = reclaim_steps(true);
        assert!(
            steps.contains(&ReclaimStep::CleanProfile { profile: "dev" }),
            "explicit reclaim plan must include cargo clean --profile dev"
        );
        assert!(
            steps.contains(&ReclaimStep::CleanProfile { profile: "release" }),
            "explicit reclaim plan must include cargo clean --profile release"
        );
        assert!(steps.contains(&ReclaimStep::PruneCargoHome));
        drop(lock);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn held_watch_lock_refusal_message_names_dev_session() {
        let root = std::env::temp_dir().join(format!(
            "sovra-xtask-clean-refuse-msg-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("target/dev")).unwrap();
        let marker = root.join("target/dev/keep-me");
        std::fs::write(&marker, b"must survive refused reclaim").unwrap();

        let holder = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(WatchLock::path(&root))
            .unwrap();
        holder.try_lock().unwrap();

        // Drive the same lock gate reclaim uses after printing size.
        let err = try_watch_lock(&root)
            .context(
                "refusing to reclaim while `cargo xtask dev` owns target/. Stop that \
                 session first, then reclaim",
            )
            .expect_err("held lock must refuse reclaim");
        let message = format!("{err:#}");
        assert!(
            message.contains("refusing to reclaim") && message.contains("watch.lock"),
            "refusal must name the lock so just clear-cache callers know why: {message}"
        );
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            b"must survive refused reclaim"
        );

        drop(holder);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cargo_home_prune_allowlist_spares_bin_and_config() {
        let home =
            std::env::temp_dir().join(format!("sovra-xtask-cargo-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for rel in [
            "bin/cargo-nextest",
            "config.toml",
            "credentials.toml",
            "registry/cache/crate.crate",
            "registry/src/crate/src/lib.rs",
            "git/db/repo",
            "git/checkouts/repo/abc",
        ] {
            let path = home.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"x").unwrap();
        }

        prune_cargo_home_caches(&home).unwrap();

        assert!(home.join("bin/cargo-nextest").is_file());
        assert!(home.join("config.toml").is_file());
        assert!(home.join("credentials.toml").is_file());
        for rel in CARGO_HOME_CACHE_DIRS {
            assert!(
                !home.join(rel).exists(),
                "allowlisted cache {rel} should be removed"
            );
        }
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn dir_size_and_budget_helpers_stay_shared() {
        assert_eq!(budget_gib().max(1), budget_gib());
        assert_eq!(budget_bytes(), budget_gib() * GIB);
        assert_eq!(dir_size(Path::new("/nonexistent/sovra/clean-cache")), 0);
        assert!(gib(GIB) > 0.9);
    }
}
