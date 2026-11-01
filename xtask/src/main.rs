//! Project automation. Run `cargo xtask help`.
//!
//! One entry point for humans, CI and AI-agent hooks, so every check runs identically
//! everywhere. Written in Rust; `setup` installs the pinned external development tools.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a CLI reports to the terminal"
)]

mod deps;
mod dev;
mod dist;
mod hook;
mod styles;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context as _, bail};

const HELP: &str = "\
cargo xtask <command>

  setup        Install pinned CI tools and the Watchexec development runner
  dev          Watch files, rebuild/relaunch the app, and stream logs (leave running)
  run          Run the desktop app (debug build)
  ci           Every gate CI runs: fmt, clippy, check-deps, check-styles, deny, shear, tests
  fix          Auto-fix: cargo fmt + clippy --fix
  check-deps   Enforce the crate layering rules (see xtask/src/deps.rs)
  check-styles Enforce the UI colour boundary and single SVG brand asset
  hook         Agent post-edit hook: reads hook JSON on stdin, checks the edited file
  dist         Build and verify a single native installer (requires pinned Velopack)
";

/// Development tools; the CI subset matches `.github/workflows/ci.yml`.
const TOOLS: &[&str] = &[
    "cargo-deny@0.20.2",
    "cargo-nextest@0.9.149",
    "cargo-shear@1.14.0",
    dev::WATCH_TOOL,
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(args: &[String]) -> anyhow::Result<ExitCode> {
    match args.first().map(String::as_str) {
        Some("setup") => setup()?,
        Some("dev") => dev::run(&root()?)?,
        Some("dev-cycle") => dev::cycle(&root()?)?, // Internal: one watcher rebuild.
        Some("run") => cargo(&["run", "--package", "sovra-app"])?,
        Some("ci") => ci()?,
        Some("fix") => fix()?,
        Some("check-deps") => deps::check(&root()?)?,
        Some("check-styles") => styles::check(&root()?)?,
        Some("hook") => return Ok(hook::run(&root()?)),
        Some("dist") => dist::run(&root()?)?,
        Some("help" | "-h" | "--help") | None => print!("{HELP}"),
        Some(other) => bail!("unknown command `{other}`\n\n{HELP}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn setup() -> anyhow::Result<()> {
    if has_cargo_subcommand("binstall") {
        cargo(&[&["binstall", "--no-confirm", "--locked"], TOOLS].concat())
    } else {
        eprintln!("cargo-binstall not found; building tools from source (slower)");
        cargo(&[&["install", "--locked"], TOOLS].concat())
    }
}

fn ci() -> anyhow::Result<()> {
    step("fmt", || cargo(&["fmt", "--all", "--check"]))?;
    step("clippy", || {
        cargo(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
    })?;
    step("check-deps", || deps::check(&root()?))?;
    step("check-styles", || styles::check(&root()?))?;
    step("deny", || cargo(&["deny", "--all-features", "check"]))?;
    step("shear", || cargo(&["shear"]))?;
    step("test", || {
        if has_cargo_subcommand("nextest") {
            cargo(&["nextest", "run", "--workspace", "--no-tests", "pass"])
        } else {
            cargo(&["test", "--workspace"])
        }
    })?;
    println!("\nxtask ci: all gates passed");
    Ok(())
}

fn fix() -> anyhow::Result<()> {
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--fix",
        "--allow-dirty",
        "--allow-staged",
    ])?;
    cargo(&["fmt", "--all"])
}

fn step(name: &str, f: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<()> {
    println!("\n==> {name}");
    f().with_context(|| format!("`{name}` failed"))
}

/// Shared target directories can supply a binary compiled in another checkout.
/// Resolve the invocation workspace rather than trusting its compiled-in path.
fn root() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().context("read xtask invocation directory")?;
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
    invocation_root(&cwd, manifest.as_deref())
}

fn invocation_root(cwd: &Path, manifest: Option<&Path>) -> anyhow::Result<PathBuf> {
    cwd.ancestors()
        .chain(manifest.into_iter().flat_map(Path::ancestors))
        .find(|path| {
            path.join("Cargo.toml").is_file()
                && path.join("xtask/Cargo.toml").is_file()
                && path.join("crates/sovra-core/Cargo.toml").is_file()
        })
        .map(Path::to_path_buf)
        .context("cannot locate the Sovra workspace; run xtask from its checkout")
}

fn cargo(args: &[&str]) -> anyhow::Result<()> {
    run(Command::new(env!("CARGO")).args(args).current_dir(root()?))
}

fn run(command: &mut Command) -> anyhow::Result<()> {
    // `cargo run -p xtask` exports xtask's package metadata. Passing that into
    // another Cargo invocation dirties build-script fingerprints (notably ring)
    // whenever a developer switches between xtask and ordinary Cargo commands.
    // Keep user Cargo configuration, target directories and logging intact.
    let package_environment: Vec<_> = std::env::vars_os()
        .map(|(name, _)| name)
        .chain(command.get_envs().map(|(name, _)| name.to_os_string()))
        .filter(|name| {
            name.to_str().is_some_and(|name| {
                name.starts_with("CARGO_PKG_")
                    || matches!(
                        name,
                        "CARGO_MANIFEST_DIR"
                            | "CARGO_MANIFEST_PATH"
                            | "CARGO_MANIFEST_LINKS"
                            | "CARGO_CRATE_NAME"
                            | "CARGO_BIN_NAME"
                            | "CARGO_PRIMARY_PACKAGE"
                            | "OUT_DIR"
                    )
            })
        })
        .collect();
    for name in package_environment {
        command.env_remove(name);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to spawn {command:?}"))?;
    if !status.success() {
        bail!("{command:?} exited with {status}");
    }
    Ok(())
}

fn has_cargo_subcommand(name: &str) -> bool {
    Command::new(env!("CARGO"))
        .args([name, "--help"])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_xtask_binary_uses_the_invocation_workspace() {
        let base =
            std::env::temp_dir().join(format!("sovra-xtask-workspaces-{}", std::process::id()));
        for name in ["first", "second"] {
            let workspace = base.join(name);
            std::fs::create_dir_all(workspace.join("xtask/src")).unwrap();
            std::fs::create_dir_all(workspace.join("crates/sovra-core")).unwrap();
            for manifest in [
                "Cargo.toml",
                "xtask/Cargo.toml",
                "crates/sovra-core/Cargo.toml",
            ] {
                std::fs::write(workspace.join(manifest), "").unwrap();
            }
        }
        std::fs::create_dir_all(base.join("outside")).unwrap();
        let base = base.canonicalize().unwrap();
        let first = base.join("first");
        let second = base.join("second");
        let outside = base.join("outside");
        for (cwd, manifest, expected) in [
            (first.join("xtask/src"), second.join("xtask"), &first),
            (
                second.join("crates/sovra-core"),
                first.join("xtask"),
                &second,
            ),
            (outside.clone(), first.join("xtask"), &first),
        ] {
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "tests::inspect_invocation_root"])
                .current_dir(cwd)
                .env("CARGO_MANIFEST_DIR", manifest)
                .env("SOVRA_XTASK_ROOT_PROBE", expected)
                .status()
                .unwrap();
            assert!(
                status.success(),
                "shared binary selected the wrong workspace"
            );
        }
        assert!(invocation_root(&outside, None).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn inspect_invocation_root() {
        let Some(expected) = std::env::var_os("SOVRA_XTASK_ROOT_PROBE") else {
            return;
        };
        assert_eq!(root().unwrap(), PathBuf::from(expected));
    }

    #[test]
    fn child_cargo_does_not_inherit_xtask_metadata() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::inspect_child_environment"])
            .env("SOVRA_XTASK_ENV_PROBE", "1")
            .env("CARGO_MANIFEST_DIR", "parent-xtask-directory")
            .env("CARGO_PKG_NAME", "xtask")
            .env("OUT_DIR", "parent-build-output")
            .env("CARGO_BUILD_TARGET", "example-target")
            .env("CARGO_TARGET_DIR", "example-target-directory")
            .env("CARGO_NET_OFFLINE", "true")
            .env("RUST_LOG", "sovra=debug");
        run(&mut command).unwrap();
    }

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "this test exists to probe the child process environment"
    )]
    fn inspect_child_environment() {
        if std::env::var_os("SOVRA_XTASK_ENV_PROBE").is_none() {
            return;
        }
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_none());
        assert!(std::env::var_os("CARGO_PKG_NAME").is_none());
        assert!(std::env::var_os("OUT_DIR").is_none());
        assert_eq!(
            std::env::var("CARGO_TARGET_DIR").unwrap(),
            "example-target-directory"
        );
        assert_eq!(
            std::env::var("CARGO_BUILD_TARGET").unwrap(),
            "example-target"
        );
        assert_eq!(std::env::var("CARGO_NET_OFFLINE").unwrap(), "true");
        assert_eq!(std::env::var("RUST_LOG").unwrap(), "sovra=debug");
    }
}
