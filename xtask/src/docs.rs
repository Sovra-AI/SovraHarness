//! Docs front-door guardrails: mermaid graphs in the map files, and relative
//! markdown links in `README.md` plus top-level `docs/*.md` must resolve.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

const MERMAID_FILES: &[&str] = &[
    "README.md",
    "docs/architecture.md",
    "docs/memory-capture-and-recall.md",
];

/// Tokens the README mermaid blocks must name so the graphs stay the real
/// layering and protocol, not a second invented architecture.
const README_MERMAID_NEEDLES: &[&str] = &[
    "sovra-core",
    "sovra-ui",
    "sovra-runtime",
    "sovra-app",
    "Command",
    "Event",
    "RunPrompt",
];

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for relative in MERMAID_FILES {
        if let Err(error) = require_mermaid(&root.join(relative), relative) {
            errors.push(error.to_string());
        }
    }
    match read(root.join("README.md")) {
        Ok(source) => {
            if let Err(error) = require_readme_mermaid_needles(&source) {
                errors.push(error.to_string());
            }
        }
        Err(error) => errors.push(error.to_string()),
    }
    if let Err(error) = check_links(root, &root.join("README.md")) {
        errors.push(error.to_string());
    }
    let docs = root.join("docs");
    for entry in fs::read_dir(&docs).with_context(|| format!("read {}", docs.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "md")
            && path.is_file()
            && let Err(error) = check_links(root, &path)
        {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    let pitch = root.join("docs/decks/pitch/index.html");
    if pitch.exists()
        && let Err(error) = check_pitch_slide4(&pitch)
    {
        bail!("{error}");
    }
    let doc_count = fs::read_dir(docs)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.path().extension().is_some_and(|ext| ext == "md") && entry.path().is_file()
        })
        .count();
    println!(
        "check-docs: mermaid graphs in README, architecture, and memory-capture-and-recall; {doc_count} top-level docs/*.md links resolve; pitch slide 4 names the sovereign-stack play"
    );
    Ok(())
}

fn require_mermaid(path: &Path, relative: &str) -> anyhow::Result<()> {
    let source = read(path)?;
    if mermaid_blocks(&source).is_empty() {
        bail!("{relative}: add a GitHub mermaid fence (```mermaid) so the architecture is visible");
    }
    Ok(())
}

fn require_readme_mermaid_needles(source: &str) -> anyhow::Result<()> {
    let joined = mermaid_blocks(source).join("\n");
    let missing: Vec<&str> = README_MERMAID_NEEDLES
        .iter()
        .copied()
        .filter(|needle| !joined.contains(needle))
        .collect();
    if !missing.is_empty() {
        bail!(
            "README.md mermaid graphs must name crate layering and the protocol ({})",
            missing.join(", ")
        );
    }
    Ok(())
}

fn mermaid_blocks(source: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```mermaid") {
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            blocks.push(body);
        }
    }
    blocks
}

fn check_links(root: &Path, file: &Path) -> anyhow::Result<()> {
    let source = read(file)?;
    let relative = file.strip_prefix(root).unwrap_or(file);
    let base = file.parent().unwrap_or(root);
    let mut missing = Vec::new();
    for (line, url) in markdown_links(&source) {
        if let Some(target) = local_markdown_target(&url) {
            let resolved = if Path::new(target).is_absolute() {
                PathBuf::from(target)
            } else {
                base.join(target)
            };
            if !resolved.exists() {
                missing.push(format!(
                    "{}:{line}: relative link `{url}` does not resolve to an existing path",
                    relative.display()
                ));
            }
        }
    }
    if !missing.is_empty() {
        bail!("{}", missing.join("\n"));
    }
    Ok(())
}

fn markdown_links(source: &str) -> Vec<(usize, String)> {
    let mut links = Vec::new();
    let mut in_fence = false;
    for (index, line) in source.lines().enumerate() {
        let line_no = index + 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        collect_inline_links(line, line_no, &mut links);
        if let Some(url) = reference_definition(line) {
            links.push((line_no, url));
        }
    }
    links
}

fn collect_inline_links(line: &str, line_no: usize, out: &mut Vec<(usize, String)>) {
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        let Some(end) = after.find(')') else {
            break;
        };
        out.push((line_no, trim_link_url(&after[..end])));
        rest = &after[end + 1..];
    }
}

fn reference_definition(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let close = trimmed.find("]:")?;
    let url = trimmed[close + 2..].trim();
    if url.is_empty() || url.contains(' ') && !url.starts_with('<') {
        // Allow `<path with space>` later; a title after the URL is common:
        // `[id]: path "title"`
        let token = url.split_whitespace().next().unwrap_or("");
        if token.is_empty() {
            return None;
        }
        return Some(trim_link_url(token));
    }
    Some(trim_link_url(url))
}

fn trim_link_url(url: &str) -> String {
    url.trim()
        .trim_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

fn local_markdown_target(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if url.split_once(':').is_some_and(|(scheme, _)| {
        matches!(scheme, "http" | "https" | "mailto" | "ftp" | "irc" | "data")
    }) {
        return None;
    }
    let path = url.split_once('#').map_or(url, |(path, _)| path);
    if path.is_empty() {
        return None;
    }
    Some(path)
}

fn read(path: impl AsRef<Path>) -> anyhow::Result<String> {
    let path = path.as_ref();
    fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

/// Fourth Cinema slide of the investor pitch: owning the sovereign stack is the play.
fn check_pitch_slide4(path: &Path) -> anyhow::Result<()> {
    let html = read(path)?;
    let slide = nth_slide(&html, 4).ok_or_else(|| {
        anyhow::anyhow!(
            "{}: missing fourth <section class=\"slide\">",
            path.display()
        )
    })?;
    pitch_slide4_play_holds(slide)
}

fn nth_slide(html: &str, n: usize) -> Option<&str> {
    let mut rest = html;
    let mut found = 0usize;
    while let Some(start) = rest.find("<section") {
        let after = &rest[start..];
        let end = after.find("</section>")? + "</section>".len();
        let block = &after[..end];
        rest = &after[end..];
        if block.contains("class=\"slide") || block.contains("class='slide") {
            found += 1;
            if found == n {
                return Some(block);
            }
        }
    }
    None
}

fn visible_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_own_word(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let mut word = String::new();
    for ch in lower.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphabetic() {
            word.push(ch);
            continue;
        }
        if matches!(word.as_str(), "own" | "owns" | "owned" | "owning") {
            return true;
        }
        word.clear();
    }
    false
}

fn pitch_slide4_play_holds(slide: &str) -> anyhow::Result<()> {
    let visible = visible_text(slide);
    let lower = visible.to_ascii_lowercase();
    if !lower.contains("sovereign stack") {
        bail!("pitch slide 4 must name the sovereign stack in visible copy; got: {visible}");
    }
    if !contains_own_word(&lower) {
        bail!("pitch slide 4 must say own/owning the sovereign stack; got: {visible}");
    }
    if !(lower.contains("the play")
        || lower.contains("this is the bet")
        || lower.contains("the bet after"))
    {
        bail!("pitch slide 4 must call owning the stack the play (or the bet); got: {visible}");
    }
    for layer in ["compute", "models", "talent", "policy", "desk"] {
        if !lower.contains(layer) {
            bail!("pitch slide 4 must name stack layer `{layer}`; got: {visible}");
        }
    }
    if !visible.contains("Sovra") {
        bail!("pitch slide 4 must name Sovra on the desk layer; got: {visible}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives in the workspace")
            .to_path_buf()
    }

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sovra-xtask-docs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(dir.join("docs")).unwrap();
        dir
    }

    fn write_minimal_graphs(root: &Path) {
        let mermaid = "```mermaid\nflowchart TB\n  app[\"sovra-app\"] --> ui[\"sovra-ui\"]\n  ui --> runtime[\"sovra-runtime\"]\n  runtime --> core[\"sovra-core\"]\n  ui -->|Command RunPrompt| runtime\n  runtime -->|Event| ui\n```\n";
        fs::write(
            root.join("README.md"),
            format!("# App\n\n{mermaid}\nSee [architecture](docs/architecture.md).\n"),
        )
        .unwrap();
        fs::write(
            root.join("docs/architecture.md"),
            format!("# Architecture\n\n{mermaid}"),
        )
        .unwrap();
        fs::write(
            root.join("docs/memory-capture-and-recall.md"),
            format!("# Memory\n\n{mermaid}"),
        )
        .unwrap();
    }

    #[test]
    fn workspace_readme_and_docs_pass_the_shipped_check() {
        check(&workspace_root()).expect("README and top-level docs must satisfy check-docs");
    }

    #[test]
    fn missing_mermaid_fails() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(root.join("README.md"), "# App\n\nNo graph.\n").unwrap();
        let error = check(&root).expect_err("README without mermaid must fail");
        assert!(
            error.to_string().contains("mermaid"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn mermaid_without_protocol_names_fails() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(
            root.join("README.md"),
            "# App\n\n```mermaid\nflowchart LR\n  a-->b\n```\n",
        )
        .unwrap();
        let error = check(&root).expect_err("README mermaid must name the protocol");
        assert!(
            error.to_string().contains("Command") || error.to_string().contains("protocol"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn broken_relative_link_fails() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(
            root.join("docs/architecture.md"),
            "# Architecture\n\n```mermaid\nflowchart LR\n  a-->b\n```\n\nSee [missing](nope.md).\n",
        )
        .unwrap();
        let error = check(&root).expect_err("broken relative link must fail");
        assert!(
            error.to_string().contains("nope.md"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn http_links_and_same_file_anchors_are_ignored() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(
            root.join("docs/architecture.md"),
            "# Architecture\n\n```mermaid\nflowchart LR\n  a-->b\n```\n\nSee [web](https://example.com) and [here](#architecture).\n",
        )
        .unwrap();
        check(&root).expect("http and fragment-only links are not files");
    }

    #[test]
    fn links_inside_code_fences_are_ignored() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(
            root.join("docs/architecture.md"),
            "# Architecture\n\n```mermaid\nflowchart LR\n  a-->b\n```\n\n```md\n[missing](nope.md)\n```\n",
        )
        .unwrap();
        check(&root).expect("fenced example links are not live links");
    }

    #[test]
    fn anchor_suffix_still_resolves_the_file() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::write(
            root.join("docs/architecture.md"),
            "# Architecture\n\n```mermaid\nflowchart LR\n  a-->b\n```\n\nSee [memory](memory-capture-and-recall.md#one-ordinary-chat-turn).\n",
        )
        .unwrap();
        check(&root).expect("fragment after a real path still resolves");
    }

    #[test]
    fn pitch_slide_4_states_owning_the_sovereign_stack_is_the_play() {
        let html = fs::read_to_string(workspace_root().join("docs/decks/pitch/index.html"))
            .expect("shipped pitch deck");
        let slide = nth_slide(&html, 4).expect("fourth <section class=\"slide\">");
        pitch_slide4_play_holds(slide)
            .expect("slide 4 visible copy must name owning the sovereign stack as the play");
        let visible = visible_text(slide);
        assert!(
            visible.contains("National compute"),
            "compute layer missing: {visible}"
        );
        assert!(
            visible.contains("National models"),
            "models layer missing: {visible}"
        );
        assert!(
            visible.contains("Talent and policy"),
            "talent/policy layer missing: {visible}"
        );
        assert!(
            visible.contains("The desk") && visible.contains("Sovra"),
            "desk/Sovra layer missing: {visible}"
        );
    }
}
