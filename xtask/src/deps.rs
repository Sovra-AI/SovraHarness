//! Crate layering rules, checked against `cargo metadata`.
//!
//! This table *is* the architecture. Dependencies point inward toward `sovra-core`;
//! adding a crate or an edge means editing this table in the same change, with a reason.

use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, bail};
use serde_json::Value;

/// Allowed internal dependencies (normal, build and dev) for each workspace crate.
const LAYERS: &[(&str, &[&str])] = &[
    ("sovra-core", &[]),
    ("sovra-adk", &["sovra-core"]),
    // Document codecs and managed attachment IO are a separate adapter dependency
    // set. Runtime receives the core FileStore port from the composition root.
    ("sovra-files", &["sovra-core"]),
    // Skill package IO and bounded YAML parsing stay behind the core SkillStore port.
    ("sovra-skills", &["sovra-core"]),
    // Google sign-in, keychain-held tokens and Google REST are their own IO boundary
    // behind the core GoogleConnector port; runtime receives it from the composition root.
    ("sovra-google", &["sovra-core"]),
    // Provider HTTP, catalog persistence and OS credentials form an IO boundary;
    // core stays pure, and UI receives only protocol data through runtime.
    ("sovra-providers", &["sovra-core"]),
    (
        "sovra-runtime",
        &["sovra-core", "sovra-adk", "sovra-providers"],
    ),
    ("sovra-store-surreal", &["sovra-core"]),
    // The WhatsApp Web client (its own crypto, protobuf and credential-store dependency
    // set) implements the core `MessagingChannel` port; runtime only sees the port.
    ("sovra-whatsapp", &["sovra-core"]),
    // Telegram's official Bot API over HTTPS; same core port, its own HTTP dependency set.
    ("sovra-telegram", &["sovra-core"]),
    ("sovra-ui", &["sovra-core", "sovra-runtime"]),
    (
        "sovra-app",
        &[
            "sovra-core",
            "sovra-files",
            "sovra-google",
            "sovra-providers",
            "sovra-runtime",
            "sovra-skills",
            "sovra-store-surreal",
            "sovra-telegram",
            "sovra-ui",
            "sovra-whatsapp",
        ],
    ),
    ("xtask", &[]),
];

/// External crates that exactly one workspace crate may depend on directly.
const EXTERNAL_OWNERS: &[(&str, &str, &str)] = &[
    (
        "tokio",
        "sovra-runtime",
        "async lives in sovra-runtime; the UI talks to it over channels",
    ),
    (
        "rmcp",
        "sovra-runtime",
        "incoming MCP sessions own async IO and cancellation in the runtime",
    ),
    ("gpui-kit", "sovra-ui", "GPUI is confined to sovra-ui"),
    (
        "whatsapp-rust",
        "sovra-whatsapp",
        "the WhatsApp Web client is confined to the sovra-whatsapp adapter crate",
    ),
    (
        "qrcode",
        "sovra-whatsapp",
        "QR grids are produced by the WhatsApp adapter; the UI only paints the matrix",
    ),
    (
        "surrealdb",
        "sovra-store-surreal",
        "the database is an implementation detail behind the MemoryStore port",
    ),
    (
        "velopack",
        "sovra-app",
        "installer/updater hooks belong to the composition root",
    ),
    (
        "adk-core",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-agent",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-model",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-runner",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-session",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-tool",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
    (
        "adk-ui",
        "sovra-adk",
        "ADK-Rust is confined to the sovra-adk adapter crate",
    ),
];

/// External crates `sovra-core` may use: pure, no IO, no async runtime, no GUI.
/// `serde_json` parses search source JSON (Mwmbl, Wikipedia, `SearXNG`) for ranking. It performs no IO.
const CORE_ALLOWED_EXTERNAL: &[&str] = &["serde", "serde_json", "thiserror", "async-trait"];

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .context("failed to run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let packages = metadata["packages"]
        .as_array()
        .context("no packages in metadata")?;
    let members: Vec<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();

    let mut violations = Vec::new();
    for package in packages {
        let name = package["name"].as_str().unwrap_or_default();
        let Some((_, allowed)) = LAYERS.iter().find(|(crate_name, _)| *crate_name == name) else {
            violations.push(format!(
                "`{name}` is not in LAYERS (xtask/src/deps.rs): declare which crates it may depend on"
            ));
            continue;
        };
        for dep in package["dependencies"].as_array().into_iter().flatten() {
            let dep_name = dep["name"].as_str().unwrap_or_default();
            let is_dev = dep["kind"].as_str() == Some("dev");
            if members.contains(&dep_name) {
                if !allowed.contains(&dep_name) {
                    violations.push(format!(
                        "`{name}` must not depend on `{dep_name}` (allowed: {allowed:?})"
                    ));
                }
            } else if let Some((_, owner, reason)) = EXTERNAL_OWNERS
                .iter()
                .find(|(krate, owner, _)| *krate == dep_name && *owner != name && !is_dev)
            {
                violations.push(format!(
                    "`{name}` must not depend on `{dep_name}`; only `{owner}` may ({reason})"
                ));
            } else if name == "sovra-core" && !is_dev && !CORE_ALLOWED_EXTERNAL.contains(&dep_name)
            {
                violations.push(format!(
                    "`sovra-core` must stay pure; `{dep_name}` is not in CORE_ALLOWED_EXTERNAL — \
                     put IO/async/GUI code in an adapter crate"
                ));
            }
        }
        if let Some(manifest) = package["manifest_path"].as_str() {
            let text = std::fs::read_to_string(manifest)?;
            if !text.contains("[lints]\nworkspace = true") {
                violations.push(format!(
                    "`{name}` must inherit workspace lints: add `[lints]\\nworkspace = true`"
                ));
            }
        }
    }

    if violations.is_empty() {
        println!(
            "check-deps: {} crates respect the layering rules",
            packages.len()
        );
        Ok(())
    } else {
        bail!("layering violations:\n  - {}", violations.join("\n  - "))
    }
}
