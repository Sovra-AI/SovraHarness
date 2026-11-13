//! Documentation guardrails: required architecture diagrams and relative links
//! across repository Markdown, including nested guides and contribution files.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

const MERMAID_FILES: &[&str] = &[
    "README.md",
    "docs/architecture.md",
    "docs/memory-capture-and-recall.md",
];

/// The product mark already shipped with the app. The README shows this file;
/// it does not add a second logo beside it.
const README_LOGO: &str = "crates/sovra-ui/assets/branding/sovra-logo.svg";

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
            if let Err(error) = require_readme_logo(root, &source) {
                errors.push(error.to_string());
            }
        }
        Err(error) => errors.push(error.to_string()),
    }
    let mut documents = Vec::new();
    collect_markdown(root, false, &mut documents)?;
    for directory in ["docs", ".github", "crates", "tests"] {
        let path = root.join(directory);
        if path.is_dir() {
            collect_markdown(&path, true, &mut documents)?;
        }
    }
    documents.sort();
    for path in &documents {
        if let Err(error) = check_links(root, path) {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("\n"));
    }
    let doc_count = documents.len();
    println!(
        "check-docs: required architecture diagrams present; {doc_count} repository Markdown files have resolving relative links; README logo {README_LOGO} has an accessible name"
    );
    Ok(())
}

fn collect_markdown(
    directory: &Path,
    recursive: bool,
    documents: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    for entry in fs::read_dir(directory).with_context(|| format!("read {}", directory.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        // Never follow a symlink into generated output or outside the checkout.
        if kind.is_dir() && recursive {
            collect_markdown(&path, true, documents)?;
        } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "md") {
            documents.push(path);
        }
    }
    Ok(())
}

fn require_mermaid(path: &Path, relative: &str) -> anyhow::Result<()> {
    let source = read(path)?;
    if mermaid_blocks(&source).is_empty() {
        bail!("{relative}: add a GitHub mermaid fence (```mermaid) so the architecture is visible");
    }
    Ok(())
}

fn require_readme_logo(root: &Path, source: &str) -> anyhow::Result<()> {
    let Some((src, alt)) = html_img_named(source, README_LOGO) else {
        bail!(
            "README.md must include an HTML <img> whose src is {README_LOGO} and whose alt text is a non-empty accessible name"
        );
    };
    if alt.trim().is_empty() {
        bail!("README.md logo <img> needs a non-empty accessible name (alt)");
    }
    let path = root.join(&src);
    if !path.is_file() {
        bail!(
            "README.md logo src `{src}` does not resolve to {}",
            path.display()
        );
    }
    Ok(())
}

/// The `<img>` whose `src` is the shipped logo, and that image's alt text.
fn html_img_named(source: &str, src_value: &str) -> Option<(String, String)> {
    let mut rest = source;
    while let Some(start) = rest.find("<img") {
        let tag = &rest[start..];
        let end = tag.find('>').map_or(tag.len(), |index| index + 1);
        let tag = &tag[..end];
        let src = html_attr(tag, "src");
        let alt = html_attr(tag, "alt");
        if src.as_deref() == Some(src_value) {
            return Some((src_value.to_string(), alt.unwrap_or_default()));
        }
        rest = &rest[start + end..];
    }
    None
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let start = tag.find(&needle)? + needle.len();
    let value = &tag[start..];
    let bytes = value.as_bytes();
    let quote = *bytes.first()?;
    if quote == b'"' || quote == b'\'' {
        let inner = &value[1..];
        let end = inner.find(quote as char)?;
        return Some(inner[..end].to_string());
    }
    let end = value.find(char::is_whitespace).unwrap_or(value.len());
    Some(value[..end].trim_end_matches('>').to_string())
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
        let logo = root.join("crates/sovra-ui/assets/branding");
        fs::create_dir_all(&logo).unwrap();
        fs::write(
            logo.join("sovra-logo.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .unwrap();
        let mermaid = "```mermaid\nflowchart TB\n  app[\"sovra-app\"] --> ui[\"sovra-ui\"]\n  ui --> runtime[\"sovra-runtime\"]\n  runtime --> core[\"sovra-core\"]\n  ui -->|Command RunPrompt| runtime\n  runtime -->|Event| ui\n```\n";
        fs::write(
            root.join("README.md"),
            format!(
                "<img src=\"crates/sovra-ui/assets/branding/sovra-logo.svg\" alt=\"Sovra logo\">\n\n# App\n\n{mermaid}\nSee [architecture](docs/architecture.md).\n"
            ),
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
    fn shipped_readme_html_header_uses_the_product_logo() {
        let root = workspace_root();
        let readme = root.join("README.md");
        let source = fs::read_to_string(&readme).expect("shipped README");
        require_readme_logo(&root, &source).expect("shipped README logo");
        let logo = root.join(README_LOGO);
        assert!(logo.is_file(), "shipped logo missing at {}", logo.display());
        eprintln!(
            "checked shipped README {} for HTML img src {README_LOGO} with a non-empty alt; file {}",
            readme.display(),
            logo.display()
        );
    }

    #[test]
    fn readme_logo_without_an_accessible_name_fails() {
        let root = scratch();
        let logo = root.join(README_LOGO);
        fs::create_dir_all(logo.parent().expect("logo parent")).unwrap();
        fs::write(&logo, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        let source = format!("<img src=\"{README_LOGO}\" alt=\"\">\n");
        let error = require_readme_logo(&root, &source).expect_err("empty alt must fail");
        assert!(error.to_string().contains("accessible name"), "{error:#}");
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
    fn nested_guides_and_contributor_files_are_checked() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::create_dir_all(root.join("docs/nested")).unwrap();
        fs::write(root.join("docs/nested/guide.md"), "[missing](missing.md)\n").unwrap();
        fs::write(root.join("CONTRIBUTING.md"), "[missing](missing.md)\n").unwrap();
        let error = check(&root).expect_err("nested and contributor links must be checked");
        let message = error.to_string();
        assert!(message.contains("docs/nested/guide.md"), "{message}");
        assert!(message.contains("CONTRIBUTING.md"), "{message}");
    }

    #[test]
    fn nested_links_resolve_from_their_own_directory() {
        let root = scratch();
        write_minimal_graphs(&root);
        fs::create_dir_all(root.join("docs/nested")).unwrap();
        fs::write(
            root.join("docs/nested/guide.md"),
            "[architecture](../architecture.md)\n",
        )
        .unwrap();
        check(&root).expect("nested relative links use the guide's directory");
    }
}
