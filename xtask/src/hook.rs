//! Post-edit hook shared by Claude Code (`PostToolUse`) and Cursor (`afterFileEdit`).
//!
//! Reads the hook's JSON payload on stdin, then for the edited file:
//! - `*.rs`: rustfmt it, then clippy its crate with warnings denied;
//! - `Cargo.toml`: re-check the crate layering rules.
//!
//! Exit code 2 + stderr is the Claude Code convention for "feed this back to the agent".

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

const FEEDBACK: u8 = 2;
const MAX_FEEDBACK_LINES: usize = 80;

pub(crate) fn run(root: &Path) -> ExitCode {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return ExitCode::SUCCESS;
    }
    let Some(file) = edited_file(&input).map(|f| absolute(root, &f)) else {
        return ExitCode::SUCCESS;
    };
    if !file.starts_with(root) || file.components().any(|c| c.as_os_str() == "target") {
        return ExitCode::SUCCESS;
    }

    let result = match file.extension().and_then(|e| e.to_str()) {
        Some("rs") => check_rust_file(root, &file)
            .and_then(|()| crate::styles::check(root).map_err(|e| format!("{e:#}"))),
        Some("svg") => crate::styles::check(root).map_err(|e| format!("{e:#}")),
        _ if file.file_name().is_some_and(|n| n == "Cargo.toml") => {
            crate::deps::check(root).map_err(|e| format!("{e:#}"))
        }
        _ => Ok(()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(feedback) => {
            eprintln!(
                "Architecture/lint check failed for {}. Fix the code; never loosen lints or \
                 the rules in xtask/src/deps.rs to make this pass.\n{feedback}",
                file.display()
            );
            ExitCode::from(FEEDBACK)
        }
    }
}

/// Claude Code sends `tool_input.file_path`; Cursor sends `file_path`.
fn edited_file(input: &str) -> Option<PathBuf> {
    let payload: Value = serde_json::from_str(input).ok()?;
    payload["tool_input"]["file_path"]
        .as_str()
        .or_else(|| payload["file_path"].as_str())
        .map(PathBuf::from)
}

fn absolute(root: &Path, file: &Path) -> PathBuf {
    if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    }
}

fn check_rust_file(root: &Path, file: &Path) -> Result<(), String> {
    // Formatting is applied, not reported: it is never a judgement call.
    let _ = Command::new("rustfmt").arg(file).current_dir(root).status();

    let Some(package) = owning_package(file) else {
        return Ok(());
    };
    let output = Command::new(env!("CARGO"))
        .args([
            "clippy",
            "--quiet",
            "--message-format",
            "short",
            "--all-targets",
        ])
        .args(["--package", &package, "--", "-D", "warnings"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("failed to run clippy: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(MAX_FEEDBACK_LINES)..];
    Err(tail.join("\n"))
}

/// Name of the nearest enclosing package, read from its `Cargo.toml`.
fn owning_package(file: &Path) -> Option<String> {
    file.ancestors().skip(1).find_map(|dir| {
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        let package = manifest.split("[package]").nth(1)?;
        package.lines().find_map(|line| {
            let value = line.trim().strip_prefix("name")?.trim().strip_prefix('=')?;
            Some(value.trim().trim_matches('"').to_owned())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_and_cursor_payloads() {
        let claude = r#"{"tool_name":"Edit","tool_input":{"file_path":"/r/a.rs"}}"#;
        let cursor = r#"{"file_path":"/r/b.rs","edits":[]}"#;
        assert_eq!(edited_file(claude), Some(PathBuf::from("/r/a.rs")));
        assert_eq!(edited_file(cursor), Some(PathBuf::from("/r/b.rs")));
        assert_eq!(edited_file("not json"), None);
    }

    #[test]
    fn finds_owning_package() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hook.rs");
        assert_eq!(owning_package(&file).as_deref(), Some("xtask"));
    }
}
