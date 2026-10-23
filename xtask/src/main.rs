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
  dist         Release build, then package with Velopack `vpk` if installed
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
    let result = match args.first().map(String::as_str) {
        Some("setup") => setup(),
        Some("dev") => dev::run(&root()),
        Some("run") => cargo(&["run", "--package", "sovra-app"]),
        Some("ci") => ci(),
        Some("fix") => fix(),
        Some("check-deps") => deps::check(&root()),
        Some("check-styles") => styles::check(&root()),
        Some("hook") => return hook::run(&root()),
        Some("dist") => dist(),
        Some("help" | "-h" | "--help") | None => {
            print!("{HELP}");
            Ok(())
        }
        Some(other) => Err(anyhow::anyhow!("unknown command `{other}`\n\n{HELP}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error:#}");
            ExitCode::FAILURE
        }
    }
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
    step("check-deps", || deps::check(&root()))?;
    step("check-styles", || styles::check(&root()))?;
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

/// Mirrors the `Package` job in `.github/workflows/release.yml` (unsigned, local channel).
fn dist() -> anyhow::Result<()> {
    cargo(&["build", "--release", "--package", "sovra-app"])?;
    let exe = if cfg!(windows) { "sovra.exe" } else { "sovra" };
    let pack_dir = root().join("target/pack");
    std::fs::create_dir_all(&pack_dir)?;
    std::fs::copy(root().join("target/release").join(exe), pack_dir.join(exe))?;
    if !has_program("vpk") {
        println!(
            "Built {}. Install Velopack's `vpk` (`dotnet tool install -g vpk`) to produce \
             installers; CI does this in .github/workflows/release.yml.",
            pack_dir.join(exe).display()
        );
        return Ok(());
    }
    run(Command::new("vpk").args([
        "pack",
        "--packId",
        "SovraHarness",
        "--packVersion",
        env!("CARGO_PKG_VERSION"),
        "--packDir",
        &pack_dir.to_string_lossy(),
        "--mainExe",
        exe,
        "--outputDir",
        &root().join("target/dist").to_string_lossy(),
    ]))
}

fn step(name: &str, f: impl FnOnce() -> anyhow::Result<()>) -> anyhow::Result<()> {
    println!("\n==> {name}");
    f().with_context(|| format!("`{name}` failed"))
}

/// Workspace root (the parent of this crate's manifest dir).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

fn cargo(args: &[&str]) -> anyhow::Result<()> {
    run(Command::new(env!("CARGO")).args(args).current_dir(root()))
}

fn run(command: &mut Command) -> anyhow::Result<()> {
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

fn has_program(name: &str) -> bool {
    Command::new(name)
        .arg("--help")
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_cargo_does_not_inherit_xtask_metadata() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "tests::inspect_child_environment"])
            .env("SOVRA_XTASK_ENV_PROBE", "1")
            .env("CARGO_MANIFEST_DIR", "parent-xtask-directory")
            .env("CARGO_PKG_NAME", "xtask")
            .env("CARGO_BUILD_TARGET", "example-target")
            .env("CARGO_NET_OFFLINE", "true")
            .env("RUST_LOG", "sovra=debug");
        run(&mut command).unwrap();
    }

    #[test]
    fn inspect_child_environment() {
        if std::env::var_os("SOVRA_XTASK_ENV_PROBE").is_none() {
            return;
        }
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_none());
        assert!(std::env::var_os("CARGO_PKG_NAME").is_none());
        assert_eq!(
            std::env::var("CARGO_BUILD_TARGET").unwrap(),
            "example-target"
        );
        assert_eq!(std::env::var("CARGO_NET_OFFLINE").unwrap(), "true");
        assert_eq!(std::env::var("RUST_LOG").unwrap(), "sovra=debug");
    }
}
