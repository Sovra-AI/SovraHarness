//! Keeping a specialist's sources when the orchestrator rewrites its answer.
//!
//! A specialist that searched the web cites its sources; the orchestrator then
//! writes the reply the user sees and may drop them. When that final reply has
//! no links at all, the links the specialists returned are listed under it, so
//! an answer built on the web always says where it came from. Only links that
//! appear in a specialist's reply are used; nothing is invented.

use std::collections::HashSet;

const MAX_SOURCES: usize = 5;

/// A "Sources" list for `answer`, or `None` when it already links somewhere
/// or the specialists returned no links.
pub fn sources_footer<'a>(
    answer: &str,
    specialist_replies: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if answer.contains("http://") || answer.contains("https://") {
        return None;
    }
    let mut seen = HashSet::new();
    let mut lines = Vec::new();
    for reply in specialist_replies {
        for (title, url) in links(reply) {
            if lines.len() == MAX_SOURCES {
                break;
            }
            if seen.insert(url.trim_end_matches('/').to_owned()) {
                lines.push(match title {
                    Some(title) if !title.trim().is_empty() => {
                        format!("- [{}]({url})", title.trim())
                    }
                    _ => format!("- <{url}>"),
                });
            }
        }
    }
    (!lines.is_empty()).then(|| format!("\n\nSources:\n{}", lines.join("\n")))
}

/// Markdown links `[title](url)` first, then bare `http(s)` URLs, in order.
fn links(text: &str) -> Vec<(Option<&str>, &str)> {
    let mut found = Vec::new();
    // Byte ranges of markdown links, blanked before the bare-URL scan so a
    // link is not found twice.
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find('[').map(|index| from + index) {
        let Some(close) = text[open..].find("](").map(|index| open + index) else {
            break;
        };
        let target_start = close + 2;
        let Some(target_end) = balanced_close(&text[target_start..]).map(|end| target_start + end)
        else {
            break;
        };
        let url = text[target_start..target_end].trim();
        if url.starts_with("http://") || url.starts_with("https://") {
            found.push((Some(&text[open + 1..close]), url));
            spans.push(open..=target_end);
        }
        from = target_end + 1;
    }
    // Same byte length as `text`, so every offset in `rest` is valid in `text`.
    let mut rest = String::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        if spans.iter().any(|span| span.contains(&index)) {
            rest.extend(std::iter::repeat_n(' ', character.len_utf8()));
        } else {
            rest.push(character);
        }
    }
    let separator = |c: char| c.is_whitespace() || "<>\"'".contains(c);
    let mut words = Vec::new();
    let mut word_start = None;
    for (index, character) in rest.char_indices() {
        match (separator(character), word_start) {
            (true, Some(start)) => {
                words.push(start..index);
                word_start = None;
            }
            (false, None) => word_start = Some(index),
            _ => {}
        }
    }
    if let Some(start) = word_start {
        words.push(start..rest.len());
    }
    for range in words {
        let start = range.start;
        let word = trim_url_end(&rest[range]);
        if word.starts_with("http://") || word.starts_with("https://") {
            // Re-borrow from `text` so the result outlives the scratch copy.
            let original = &text[start..start + word.len()];
            if !found.iter().any(|(_, url)| *url == original) {
                found.push((None, original));
            }
        }
    }
    found
}

/// Index of the `)` closing a markdown target, allowing `(…)` inside the URL.
fn balanced_close(target: &str) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, character) in target.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            '\n' => return None,
            _ => {}
        }
    }
    None
}

/// Drops sentence punctuation and an unbalanced closing bracket after a URL.
fn trim_url_end(word: &str) -> &str {
    let mut word = word.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    while word.ends_with(')') && word.matches(')').count() > word.matches('(').count() {
        word = word[..word.len() - 1].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    }
    word
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = "Rust 1.99.1 is the latest.\n\nSources:\n\
        - [Rust Blog](https://blog.rust-lang.org/releases)\n\
        - [Wikipedia](https://en.wikipedia.org/wiki/Rust_(programming_language))\n\
        Also see https://doc.rust-lang.org/releases.html.";

    #[test]
    fn a_linkless_answer_gets_the_specialists_sources() {
        let footer = sources_footer("The latest stable Rust is 1.99.1.", [REPLY]).expect("footer");
        assert!(
            footer.starts_with("\n\nSources:\n- [Rust Blog](https://blog.rust-lang.org/releases)")
        );
        assert!(footer.contains("- <https://doc.rust-lang.org/releases.html>"));
        assert!(footer.contains("(https://en.wikipedia.org/wiki/Rust_(programming_language))"));
        assert_eq!(footer.matches("\n- ").count(), 3, "{footer}");
    }

    #[test]
    fn an_answer_with_links_or_no_specialist_links_is_left_alone() {
        assert_eq!(sources_footer("See https://rust-lang.org", [REPLY]), None);
        assert_eq!(sources_footer("No links here.", ["Nothing to cite."]), None);
        assert_eq!(sources_footer("No links here.", []), None);
    }

    #[test]
    fn accented_titles_and_unusual_spaces_do_not_shift_links() {
        let reply = "Voir [Café é](https://cafe.example/menu) et\u{a0}https://thé.example/x.";
        let footer = sources_footer("Réponse.", [reply]).expect("footer");
        assert!(
            footer.contains("- [Café é](https://cafe.example/menu)"),
            "{footer}"
        );
        assert!(footer.contains("- <https://thé.example/x>"), "{footer}");
    }

    #[test]
    fn duplicates_collapse_and_the_list_is_capped() {
        let many = (0..9)
            .map(|index| format!("[t{index}](https://site.example/{index})"))
            .collect::<Vec<_>>()
            .join(" ");
        let doubled = format!("{many}{many}");
        let footer = sources_footer("answer", [doubled.as_str()]).expect("footer");
        assert_eq!(footer.matches("\n- ").count(), MAX_SOURCES);
    }
}
