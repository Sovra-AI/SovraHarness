//! Reply references are navigation hints, not proof of saved evidence.

use super::{EpisodeId, FactId};

/// Bound the controls attached to a model-authored reply.
pub const MAX_MEMORY_CITATIONS: usize = 32;

const MEMORY_PREFIX: &str = "[memory:";
const SOURCE_PREFIX: &str = "[source:";
const PREVIEW_LIMIT: usize = 140;

/// A user-facing row for one saved-detail reference, with no opaque token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCitationRef {
    /// Nearby wording from the reply, hashes stripped. Empty when the token stood alone.
    pub preview: String,
    /// Fact or original-source inspect target.
    pub target: MemoryCitationTarget,
}

/// Where a reply citation should open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryCitationTarget {
    Fact(FactId),
    Source(EpisodeId),
}

/// Recognize complete `[memory:fact:…]` references in first-appearance order.
/// The store must still resolve the opaque ID; this never verifies a model claim.
pub fn memory_citations(text: &str) -> Vec<FactId> {
    memory_citation_refs(text)
        .into_iter()
        .filter_map(|item| match item.target {
            MemoryCitationTarget::Fact(id) => Some(id),
            MemoryCitationTarget::Source(_) => None,
        })
        .collect()
}

/// Source-only citations navigate to evidence without inventing a fact identifier.
pub fn memory_source_citations(text: &str) -> Vec<EpisodeId> {
    memory_citation_refs(text)
        .into_iter()
        .filter_map(|item| match item.target {
            MemoryCitationTarget::Fact(_) => None,
            MemoryCitationTarget::Source(id) => Some(id),
        })
        .collect()
}

/// Saved-detail rows for an in-chat listing, in first-appearance order.
pub fn memory_citation_refs(text: &str) -> Vec<MemoryCitationRef> {
    let mut refs = Vec::new();
    let mut seen_facts: Vec<FactId> = Vec::new();
    let mut seen_sources: Vec<EpisodeId> = Vec::new();
    let mut search_from = 0;
    while refs.len() < MAX_MEMORY_CITATIONS {
        let rest = &text[search_from..];
        let Some((rel, kind)) = next_prefix(rest) else {
            break;
        };
        let token_at = search_from + rel;
        let after_open = token_at + kind.prefix().len();
        let Some(close) = text[after_open..].find(']') else {
            break;
        };
        let inner = &text[after_open..after_open + close];
        search_from = after_open + close + 1;
        let preview = preview_before(text, token_at);
        match kind {
            TokenKind::Memory if valid_id(inner, "fact:") => {
                let id = FactId(inner.into());
                if seen_facts.contains(&id) {
                    continue;
                }
                seen_facts.push(id.clone());
                refs.push(MemoryCitationRef {
                    preview,
                    target: MemoryCitationTarget::Fact(id),
                });
            }
            TokenKind::Source if valid_id(inner, "episode:") => {
                let id = EpisodeId(inner.into());
                if seen_sources.contains(&id) {
                    continue;
                }
                seen_sources.push(id.clone());
                refs.push(MemoryCitationRef {
                    preview,
                    target: MemoryCitationTarget::Source(id),
                });
            }
            TokenKind::Memory | TokenKind::Source => {}
        }
    }
    refs
}

/// Strip `[memory:…]` / `[source:…]` tokens from user-visible text.
///
/// Incomplete trailing prefixes are omitted so hashes never flash while streaming.
/// Already-kept characters are never rewritten as the source grows, so Markdown
/// can be fed the result incrementally.
pub fn hide_memory_citation_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut search_from = 0;
    while let Some((rel, kind)) = next_prefix(&text[search_from..]) {
        let token_at = search_from + rel;
        out.push_str(&text[search_from..token_at]);
        let after_open = token_at + kind.prefix().len();
        match text[after_open..].find(']') {
            Some(close) => search_from = after_open + close + 1,
            None => return out,
        }
    }
    out.push_str(&text[search_from..]);
    out
}

#[derive(Clone, Copy)]
enum TokenKind {
    Memory,
    Source,
}

impl TokenKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Memory => MEMORY_PREFIX,
            Self::Source => SOURCE_PREFIX,
        }
    }
}

fn next_prefix(text: &str) -> Option<(usize, TokenKind)> {
    let memory = text.find(MEMORY_PREFIX);
    let source = text.find(SOURCE_PREFIX);
    match (memory, source) {
        (Some(memory), Some(source)) => {
            if memory <= source {
                Some((memory, TokenKind::Memory))
            } else {
                Some((source, TokenKind::Source))
            }
        }
        (Some(memory), None) => Some((memory, TokenKind::Memory)),
        (None, Some(source)) => Some((source, TokenKind::Source)),
        (None, None) => None,
    }
}

fn valid_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() > prefix.len()
        && id.len() <= 512
        && !id
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || "[()<>".contains(ch))
}

fn preview_before(text: &str, token_at: usize) -> String {
    let before = &text[..token_at];
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let line = hide_memory_citation_tokens(&before[line_start..]);
    let line = line.trim();
    let line = line.trim_end_matches([':', '-', '—', '–', ',']).trim();
    if line.is_empty() {
        return String::new();
    }
    if line.chars().count() <= PREVIEW_LIMIT {
        return line.to_owned();
    }
    let mut clipped = String::new();
    for (index, ch) in line.chars().enumerate() {
        if index + 1 == PREVIEW_LIMIT {
            break;
        }
        clipped.push(ch);
    }
    if let Some((keep, _)) = clipped.rsplit_once(' ')
        && !keep.is_empty()
    {
        return format!("{keep}…");
    }
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn citations_require_complete_bounded_fact_references_and_keep_order() {
        assert_eq!(
            memory_citations(
                "[memory:fact:茶] [memory:fact:two] [memory:fact:茶] [memory:episode:no] [memory:fact:] [memory:fact:two words] [memory:fact:x\n] [memory:fact:[bad] [memory:fact:unfinished"
            ),
            vec![FactId("fact:茶".into()), FactId("fact:two".into())]
        );
        assert_eq!(
            memory_citations(&format!("[memory:fact:{}]", "a".repeat(512))),
            Vec::<FactId>::new()
        );
        assert_eq!(
            memory_citations("[web:fact:tea] memory:fact:tea https://example.com"),
            Vec::<FactId>::new()
        );
    }

    #[test]
    fn source_citations_remain_distinct_from_facts() {
        let text =
            "[source:episode:one] [source:episode:one] [source:fact:no] [source:episode:two words]";
        assert_eq!(
            memory_source_citations(text),
            vec![EpisodeId("episode:one".into())]
        );
        assert_eq!(memory_citations(text), Vec::<FactId>::new());
    }

    #[test]
    fn citations_bound_unique_controls_without_counting_duplicates() {
        let mut text = "[memory:fact:first] ".repeat(100);
        for index in 0..100 {
            write!(text, "[memory:fact:{index}] ").unwrap();
        }
        let ids = memory_citations(&text);
        assert_eq!(ids.len(), MAX_MEMORY_CITATIONS);
        assert_eq!(ids[0], FactId("fact:first".into()));
        assert_eq!(ids.last(), Some(&FactId("fact:30".into())));
    }

    #[test]
    fn hide_strips_complete_tokens_and_holds_an_unfinished_one() {
        let long = "memory_0565e570b3b344ef836475a27e4586b023fbc91d38c2818650159b612c2c5d57";
        let text = format!(
            "Where you live: You've mentioned living in Dubai with your girlfriend [memory:fact:{long}].\n\
             Cooking style: You like recipes to be short [source:episode:ep1].\n\
             Keep this [link](https://example.com) and [note]."
        );
        let shown = hide_memory_citation_tokens(&text);
        assert!(
            !shown.contains("memory:fact:") && !shown.contains("source:episode:"),
            "{shown}"
        );
        assert!(shown.contains("Dubai with your girlfriend"));
        assert!(shown.contains("[link](https://example.com)"));
        assert!(shown.contains("[note]"));
        assert_eq!(
            hide_memory_citation_tokens("Tea. [memory:fact:unfinished"),
            "Tea. "
        );
        let growing = format!("Tea. [memory:fact:{long}]");
        let first = hide_memory_citation_tokens(&growing[..growing.len() - 1]);
        let full = hide_memory_citation_tokens(&growing);
        assert!(full.starts_with(&first), "{first} -> {full}");
        assert_eq!(full, "Tea. ");
    }

    #[test]
    fn citation_rows_keep_ids_for_controls_and_plain_previews() {
        let long = "memory_0565e570b3b344ef836475a27e4586b023fbc91d38c2818650159b612c2c5d57";
        let text = format!(
            "Here's what I can see so far.\n\
             Where you live: You've mentioned living in Dubai with your girlfriend [memory:fact:{long}]\n\
             Food you enjoy: You like mint sauce [source:episode:ep-mint]"
        );
        let refs = memory_citation_refs(&text);
        assert_eq!(
            refs[0].target,
            MemoryCitationTarget::Fact(FactId(format!("fact:{long}")))
        );
        assert_eq!(
            refs[1].target,
            MemoryCitationTarget::Source(EpisodeId("episode:ep-mint".into()))
        );
        for item in &refs {
            assert!(
                !item.preview.contains("memory:fact:") && !item.preview.contains("source:episode:"),
                "{}",
                item.preview
            );
        }
        assert!(refs[0].preview.contains("Dubai"));
        assert!(refs[1].preview.contains("mint sauce"));
        assert_eq!(
            memory_citations(&text),
            vec![FactId(format!("fact:{long}"))]
        );
        assert_eq!(
            memory_source_citations(&text),
            vec![EpisodeId("episode:ep-mint".into())]
        );
    }
}
