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

    /// The release workflow and the workspace manifest on disk, not a copy of either.
    fn shipped_release_sources() -> (String, String) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives in the workspace")
            .to_path_buf();
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("root Cargo.toml");
        let workflow = std::fs::read_to_string(root.join(".github/workflows/release.yml"))
            .expect("shipped release workflow");
        (manifest, workflow)
    }

    fn workspace_package_version(manifest: &str) -> String {
        let mut in_package = false;
        for line in manifest.lines() {
            if line == "[workspace.package]" {
                in_package = true;
                continue;
            }
            if in_package && line.starts_with('[') {
                break;
            }
            if in_package {
                let Some(rest) = line.strip_prefix("version = ") else {
                    continue;
                };
                return rest.trim().trim_matches('"').to_string();
            }
        }
        panic!("workspace package version missing from Cargo.toml");
    }

    fn semver_triple(version: &str) -> (u64, u64, u64) {
        let mut parts = version.split('.');
        let major = parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("major");
        let minor = parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("minor");
        let patch = parts
            .next()
            .and_then(|part| part.parse().ok())
            .expect("patch");
        assert!(
            parts.next().is_none(),
            "package version is major.minor.patch"
        );
        (major, minor, patch)
    }

    #[test]
    fn main_push_builds_three_os_executables_at_the_package_version() {
        let (manifest, workflow) = shipped_release_sources();
        let version = workspace_package_version(&manifest);
        assert_ne!(
            version, "main",
            "the package version is never the branch name"
        );
        assert!(
            semver_triple(&version) > (0, 3, 0),
            "shipped version {version} must be greater than 0.3.0"
        );
        assert!(
            workflow.contains("branches: [main]"),
            "a push to main runs the release workflow"
        );
        for row in [
            "{ os: macos-15, channel: osx, exe: sovra }",
            "{ os: windows-latest, channel: win, exe: sovra.exe }",
            "{ os: ubuntu-24.04, channel: linux, exe: sovra }",
        ] {
            assert!(workflow.contains(row), "missing matrix row {row}");
        }
        assert!(workflow.contains("cargo build --release --locked --package sovra-app"));
        assert!(workflow.contains("[workspace.package]"));
        assert!(workflow.contains("version = "));
        assert!(workflow.contains("packVersion \"$version\""));
        assert!(workflow.contains("tag=\"v$version\""));
        assert!(workflow.contains("--tag \"$tag\""));
        assert!(workflow.contains("path: target/release/${{ matrix.exe }}"));
        assert!(!workflow.contains("VERSION=\"${GITHUB_REF_NAME#v}\""));
        assert!(!workflow.contains("--tag \"$GITHUB_REF_NAME\""));
        assert!(workflow.contains("HAS_MAC_SIGNING"));
        assert!(workflow.contains("HAS_WIN_SIGNING"));
        assert!(workflow.contains("runner.os == 'macOS' && env.HAS_MAC_SIGNING == 'true'"));
        assert!(workflow.contains("runner.os == 'Windows' && env.HAS_WIN_SIGNING == 'true'"));
    }
}
