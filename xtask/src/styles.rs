//! Lightweight design-boundary guardrails, shared by CI and agent hooks.

use std::path::Path;

use anyhow::{Context as _, bail};

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let ui = root.join("crates/sovra-ui");
    check_views(&ui.join("src"), &ui.join("src/design"))?;
    let branding = ui.join("assets/branding");
    for entry in std::fs::read_dir(&branding).context("read bundled branding")? {
        let entry = entry?;
        if entry.file_name() != "sovra-logo.svg" || !entry.file_type()?.is_file() {
            bail!(
                "{}: branding contains only sovra-logo.svg; see docs/styling.md",
                entry.path().display()
            );
        }
    }
    if !branding.join("sovra-logo.svg").is_file() {
        bail!("missing embedded Sovra logo");
    }
    Ok(())
}

fn check_views(directory: &Path, design: &Path) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path == design {
            continue;
        }
        if path.is_dir() {
            check_views(&path, design)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path)?;
            for (line, text) in source.lines().enumerate() {
                let code = text.split("//").next().unwrap_or_default();
                // Deliberately a source guard, not a Rust parser: catches common
                // literal colours, including qualified calls, without another dependency.
                let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
                if ["rgb(", "rgba(", "hsl(", "hsla(", "0x"]
                    .iter()
                    .any(|p| compact.contains(p))
                {
                    bail!(
                        "{}:{}: put raw colours in src/design and use semantic theme roles in views",
                        path.display(),
                        line + 1
                    );
                }
                if compact.contains("Button::new(") {
                    bail!(
                        "{}:{}: use design::button or design::primary_button in views so buttons get Sovra geometry and pointer cursor",
                        path.display(),
                        line + 1
                    );
                }
                if compact.contains("Checkbox::new(") && !compact.contains(".cursor_pointer()") {
                    let window_end = (line + 6).min(source.lines().count());
                    let snippet: String = source
                        .lines()
                        .skip(line)
                        .take(window_end - line)
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !snippet.contains(".cursor_pointer()") {
                        bail!(
                            "{}:{}: Checkbox::new must include .cursor_pointer()",
                            path.display(),
                            line + 1
                        );
                    }
                }
                if compact.contains("Select::new(") && !compact.contains(".cursor_pointer()") {
                    let window_end = (line + 8).min(source.lines().count());
                    let snippet: String = source
                        .lines()
                        .skip(line)
                        .take(window_end - line)
                        .collect::<Vec<_>>()
                        .join(" ");
                    if !snippet.contains(".cursor_pointer()") {
                        bail!(
                            "{}:{}: Select::new must include .cursor_pointer()",
                            path.display(),
                            line + 1
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
