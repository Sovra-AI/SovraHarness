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
            check_adaptive_sizes(&path, &source)?;
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

/// Responsive layout rules (docs/styling.md → Responsive layout): sizes come from
/// rem-based tokens so they follow the interface scale, and only the shell —
/// the window's root view — may size panes from the window instead of measuring
/// its own container. Test modules may use literal window sizes.
fn check_adaptive_sizes(path: &Path, source: &str) -> anyhow::Result<()> {
    let shell = path.file_name().is_some_and(|name| name == "shell.rs");
    for (line, text) in source.lines().enumerate() {
        if text.starts_with("mod tests") {
            break;
        }
        let code = text.split("//").next().unwrap_or_default();
        let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let literal = |call: &str| {
            compact.match_indices(call).any(|(at, _)| {
                let method = compact[..at].ends_with('.');
                let argument = &compact[at + call.len()..];
                !method && !argument.starts_with("0.)") && !argument.starts_with("0.0)")
            })
        };
        if literal("px(") || literal("rems(") {
            bail!(
                "{}:{}: use rem-based tokens from design::tokens(cx) instead of literal sizes, \
                 so layouts follow the interface scale; see docs/styling.md → Responsive layout",
                path.display(),
                line + 1
            );
        }
        if !shell && (compact.contains("viewport_size()") || compact.contains("of_window(")) {
            bail!(
                "{}:{}: size a view from its own container with design::width_probe, not the \
                 window; see docs/styling.md → Responsive layout",
                path.display(),
                line + 1
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::check_adaptive_sizes;

    fn passes(file: &str, source: &str) -> bool {
        check_adaptive_sizes(Path::new(file), source).is_ok()
    }

    #[test]
    fn literal_sizes_are_rejected_but_tokens_padding_and_zero_pass() {
        assert!(!passes("view.rs", "    .w(px(260.))"));
        assert!(!passes("view.rs", "    .gap(gpui_kit::px(2.))"));
        assert!(!passes("view.rs", "    .max_w(rems(40.))"));
        assert!(passes(
            "view.rs",
            "    .px(t.space.small).w(t.layout.sidebar_width)"
        ));
        assert!(passes(
            "view.rs",
            "    ListState::new(0, ListAlignment::Top, px(0.))"
        ));
        assert!(passes(
            "view.rs",
            "    // a .w(px(260.)) example in a comment"
        ));
        assert!(passes(
            "view.rs",
            "mod tests {\n    size(px(640.), px(420.))"
        ));
    }

    #[test]
    fn only_the_shell_reads_the_window_width() {
        let source = "    let narrow = window.viewport_size().width < limit;";
        assert!(!passes("models_view.rs", source));
        assert!(passes("shell.rs", source));
        assert!(!passes(
            "models_view.rs",
            "    WidthClass::of_window(window)"
        ));
    }
}
