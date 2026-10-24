//! Reproducible native packaging. A successful command must produce an installer.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, bail};

const VPK_VERSION: &str = "1.2.161";

pub(crate) fn run(root: &Path) -> anyhow::Result<()> {
    // Fail before a release build if the packaging dependency is unavailable.
    // An override lets CI use an isolated tool installation without changing PATH.
    let vpk = std::env::var_os("SOVRA_VPK").unwrap_or_else(|| "vpk".into());
    let version = Command::new(&vpk).arg("--help").output().with_context(|| {
        format!("Install the build-only packaging tool: dotnet tool install -g vpk --version {VPK_VERSION}; or set SOVRA_VPK to its executable")
    })?;
    let reported = String::from_utf8_lossy(&version.stdout);
    if !version.status.success()
        || !reported.lines().any(|line| {
            line.trim()
                .starts_with(&format!("Velopack CLI {VPK_VERSION},"))
        })
    {
        bail!("Expected Velopack {VPK_VERSION}; found {}", reported.trim());
    }
    let (runtime, channel, extension) = platform(std::env::consts::OS, std::env::consts::ARCH)?;
    let target = root.join("target");
    let mut build = Command::new(env!("CARGO"));
    build
        .current_dir(root)
        .args([
            "build",
            "--release",
            "--locked",
            "--package",
            "sovra-app",
            "--target-dir",
        ])
        .arg(&target);
    crate::run(&mut build)?;

    let exe = if cfg!(windows) { "sovra.exe" } else { "sovra" };
    // Each invocation has fresh input and output directories: old assets cannot
    // make a broken package look successful, and source/secrets cannot be bundled.
    let run_id = format!("{runtime}-{}", std::process::id());
    let stage = target.join("pack").join(&run_id);
    let output = target.join("dist").join(&run_id);
    std::fs::create_dir_all(stage.parent().context("package parent")?)?;
    std::fs::create_dir(&stage).context("create fresh package staging directory")?;
    std::fs::create_dir_all(output.parent().context("output parent")?)?;
    std::fs::create_dir(&output).context("create fresh installer output directory")?;
    std::fs::copy(target.join("release").join(exe), stage.join(exe))?;

    let mut package = Command::new(vpk);
    package
        .current_dir(root)
        .args([
            "pack",
            "--skip-updates",
            "--packId",
            "SovraHarness",
            "--packTitle",
            "Sovra",
            "--packVersion",
            env!("CARGO_PKG_VERSION"),
            "--mainExe",
            exe,
            "--runtime",
            runtime,
            "--channel",
            channel,
            "--packDir",
        ])
        .arg(&stage)
        .arg("--outputDir")
        .arg(&output);
    if cfg!(target_os = "macos") {
        package
            .args(["--bundleId", "com.sovra.harness", "--signEntitlements"])
            .arg(root.join("packaging/macos/sovra.entitlements"));
    }
    crate::run(&mut package)?;
    let installer = find_installer(&output, extension)?;
    println!("\nInstaller: {}", installer.display());
    println!("Local build: verify signing/notarization before distributing to other computers.");
    Ok(())
}

fn platform(os: &str, arch: &str) -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    match (os, arch) {
        ("macos", "aarch64") => Ok(("osx-arm64", "osx", "pkg")),
        ("macos", "x86_64") => Ok(("osx-x64", "osx-x64", "pkg")),
        ("windows", "x86_64") => Ok(("win-x64", "win", "exe")),
        ("windows", "aarch64") => Ok(("win-arm64", "win-arm64", "exe")),
        ("linux", "x86_64") => Ok(("linux-x64", "linux", "AppImage")),
        ("linux", "aarch64") => Ok(("linux-arm64", "linux-arm64", "AppImage")),
        _ => bail!("No supported installer target for {os}/{arch}"),
    }
}

fn find_installer(output: &Path, extension: &str) -> anyhow::Result<PathBuf> {
    let mut installers = Vec::new();
    for entry in std::fs::read_dir(output)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|value| value == extension)
            && entry.file_type()?.is_file()
            && entry.metadata()?.len() > 0
        {
            installers.push(path);
        }
    }
    if installers.len() != 1 {
        bail!(
            "Packaging did not produce exactly one nonempty .{extension} installer in {}",
            output.display()
        );
    }
    Ok(installers.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_supported_platforms_and_rejects_unknown_targets() {
        assert_eq!(
            platform("macos", "aarch64").expect("supported"),
            ("osx-arm64", "osx", "pkg")
        );
        assert_eq!(
            platform("windows", "x86_64").expect("supported"),
            ("win-x64", "win", "exe")
        );
        assert_eq!(
            platform("linux", "x86_64").expect("supported"),
            ("linux-x64", "linux", "AppImage")
        );
        assert!(platform("freebsd", "x86_64").is_err());
    }

    #[test]
    fn rejects_missing_empty_and_ambiguous_installers() {
        let dir = std::env::temp_dir().join(format!("sovra-package-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("test directory");
        assert!(find_installer(&dir, "pkg").is_err());
        std::fs::write(dir.join("Sovra.pkg"), []).expect("empty package");
        assert!(find_installer(&dir, "pkg").is_err());
        std::fs::write(dir.join("Sovra.pkg"), b"package payload").expect("test package");
        assert_eq!(
            find_installer(&dir, "pkg").expect("one installer"),
            dir.join("Sovra.pkg")
        );
        std::fs::write(dir.join("Other.pkg"), b"unexpected duplicate").expect("duplicate");
        assert!(find_installer(&dir, "pkg").is_err());
        std::fs::remove_dir_all(dir).expect("remove test directory");
    }
}
