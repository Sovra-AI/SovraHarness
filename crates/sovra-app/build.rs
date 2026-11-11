//! Bakes Sovra's Google "Desktop app" OAuth client into the binary, so people
//! press **Connect Google** with no setup.
//!
//! Values come from the build environment (CI, release pipeline) or, failing
//! that, the gitignored `google-client.env` at the workspace root (local
//! builds). Google documents a desktop client's secret as not confidential, so
//! shipping it is allowed; it still never belongs in git.

use std::path::Path;

const KEYS: [&str; 2] = ["SOVRA_GOOGLE_CLIENT_ID", "SOVRA_GOOGLE_CLIENT_SECRET"];

#[expect(
    clippy::disallowed_methods,
    reason = "a build script reads its build environment once"
)]
fn main() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../google-client.env");
    println!("cargo:rerun-if-changed={}", file.display());
    let from_file = std::fs::read_to_string(&file).unwrap_or_default();
    for key in KEYS {
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
