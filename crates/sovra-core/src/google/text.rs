//! Turning outside text (emails, documents) into something safe and short for a model.

/// Cuts `text` to at most `max_chars` characters on a word boundary, saying so.
pub fn clip_text(text: &str, max_chars: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max_chars).collect();
    let cut = cut
        .rfind(char::is_whitespace)
        .filter(|at| *at > max_chars / 2)
        .map_or(cut.as_str(), |at| &cut[..at]);
    format!("{} … [cut short]", cut.trim_end())
}

/// Readable text from an HTML email: tags dropped, blocks become line breaks,
/// scripts and styles removed, common entities decoded, blank runs collapsed.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..start]));
        let after = &rest[start + 1..];
        let starts_tag = after
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        let end = after.find('>').filter(|_| starts_tag);
        let Some(end) = end else {
            // A lone "<" (as in "a < b") is text, not the start of a tag.
            out.push('<');
            rest = after;
            continue;
        };
        let tag = after[..end].trim().to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        rest = &after[end + 1..];
        if !tag.starts_with('/') && matches!(name.as_str(), "script" | "style" | "head") {
            let close = format!("</{name}");
            rest = rest
                .to_ascii_lowercase()
                .find(&close)
                .and_then(|at| rest[at..].find('>').map(|gt| &rest[at + gt + 1..]))
                .unwrap_or("");
            continue;
        }
        if matches!(
            name.as_str(),
            "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "table" | "blockquote"
        ) {
            out.push('\n');
        }
    }
    out.push_str(&decode_entities(rest));
    collapse_blank_lines(&out)
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() && lines.last().is_none_or(String::is_empty) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}
