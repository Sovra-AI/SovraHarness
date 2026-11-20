//! Builds this repository's own code plugin (`examples/code-plugin`) into the
//! `plugin.wasm` that sits beside `sovra-plugin.json` at the repository root, and
//! checks in CI that the committed file is exactly what its source builds to.
//!
//! The build is reproducible: dependencies are locked, and every absolute path
//! (toolchain, Cargo home, this checkout) is rewritten, so the published module
//! names no one's computer and the same source gives the same bytes anywhere.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

const SOURCE: &str = "examples/code-plugin";
const TARGET: &str = "wasm32-unknown-unknown";
const ARTIFACT: &str = "sovra_date_helper.wasm";
const PUBLISHED: &str = "plugin.wasm";

/// `cargo xtask plugin-wasm`: rebuilds the module and writes it to the root.
pub(crate) fn write(root: &Path) -> anyhow::Result<()> {
    let built = build(root)?;
    std::fs::copy(&built, root.join(PUBLISHED)).context("write plugin.wasm")?;
    println!("wrote {PUBLISHED} from {SOURCE}");
    Ok(())
}

/// CI step: the committed module must match a fresh build of its source.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let built = std::fs::read(build(root)?).context("read the fresh build")?;
    let published = std::fs::read(root.join(PUBLISHED)).context("read plugin.wasm")?;
    if built != published {
        bail!(
            "{PUBLISHED} does not match {SOURCE}. Run `cargo xtask plugin-wasm` and commit the result."
        );
    }
    println!("{PUBLISHED} matches {SOURCE}");
    Ok(())
}

fn build(root: &Path) -> anyhow::Result<PathBuf> {
    let source = root.join(SOURCE);
    let target_dir = root.join("target").join("code-plugin");
    let sysroot = output(
        Command::new("rustc")
            .args(["--print", "sysroot"])
            .current_dir(&source),
    )?;
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
        .context("find the Cargo home folder")?;
    // Unit separator, as Cargo expects in CARGO_ENCODED_RUSTFLAGS.
    let flags = [
        format!("--remap-path-prefix={}=/rust", sysroot.trim()),
        format!("--remap-path-prefix={}=/cargo", cargo_home.display()),
        format!("--remap-path-prefix={}=/plugin", source.display()),
    ]
    .join("\u{1f}");
    crate::run(
        Command::new(env!("CARGO"))
            .args([
                "build",
                "--release",
                "--locked",
                "--target",
                TARGET,
                "--target-dir",
            ])
            .arg(&target_dir)
            .env("CARGO_ENCODED_RUSTFLAGS", flags)
            .env_remove("RUSTFLAGS")
            .current_dir(&source),
    )
    .with_context(|| {
        format!("build {SOURCE} (install the target with `rustup target add {TARGET}`)")
    })?;
    Ok(target_dir.join(TARGET).join("release").join(ARTIFACT))
}

fn output(command: &mut Command) -> anyhow::Result<String> {
    let out = command
        .output()
        .with_context(|| format!("failed to spawn {command:?}"))?;
    if !out.status.success() {
        bail!("{command:?} exited with {}", out.status);
    }
    String::from_utf8(out.stdout).context("tool output is not text")
}
