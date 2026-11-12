//! Bakes Sovra's Google "Desktop app" OAuth client and GitHub OAuth App client id
//! into the binary, so people press **Connect Google** or **Connect GitHub** with
//! no setup.
//!
//! Values come from the build environment (CI, release pipeline) or, failing
//! that, the gitignored `google-client.env` / `github-client.env` at the workspace
//! root (local builds). Google documents a desktop client's secret as not
//! confidential, and a GitHub device-flow client id is public, so shipping them is
//! allowed; they still never belong in git.

use std::path::Path;

/// Each env file and the names it may set.
const FILES: [(&str, &[&str]); 2] = [
    (
        "google-client.env",
        &["SOVRA_GOOGLE_CLIENT_ID", "SOVRA_GOOGLE_CLIENT_SECRET"],
    ),
    ("github-client.env", &["SOVRA_GITHUB_CLIENT_ID"]),
];

#[expect(
    clippy::disallowed_methods,
    reason = "a build script reads its build environment once"
)]
fn main() {
    for (name, keys) in FILES {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(name);
        println!("cargo:rerun-if-changed={}", file.display());
        let from_file = std::fs::read_to_string(&file).unwrap_or_default();
        for key in keys {
            println!("cargo:rerun-if-env-changed={key}");
            let value = std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| lookup(&from_file, key));
            if let Some(value) = value {
                println!("cargo:rustc-env={key}={}", value.trim());
            }
        }
    }
}

/// `KEY=value` lines; `#` comments, blank lines and surrounding quotes are ignored.
fn lookup(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        if name.trim().trim_start_matches("export ").trim() != key {
            return None;
        }
        let value = value.trim().trim_matches(['"', '\'']).trim();
        (!value.is_empty()).then(|| value.to_owned())
    })
}
