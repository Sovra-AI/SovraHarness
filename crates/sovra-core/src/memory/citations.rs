//! Reply references are navigation hints, not proof of saved evidence.

use super::{EpisodeId, FactId};

/// Bound the controls attached to a model-authored reply.
pub const MAX_MEMORY_CITATIONS: usize = 32;

/// Recognize complete `[memory:fact:…]` references in first-appearance order.
/// The store must still resolve the opaque ID; this never verifies a model claim.
pub fn memory_citations(text: &str) -> Vec<FactId> {
    let mut ids = Vec::new();
    for tail in text.split("[memory:").skip(1) {
        let Some((id, _)) = tail.split_once(']') else {
            continue;
        };
        if !id.starts_with("fact:")
            || id.len() <= "fact:".len()
            || id.len() > 512
            || id
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control() || "[()<>".contains(ch))
        {
            continue;
        }
        let id = FactId(id.into());
        if !ids.contains(&id) {
            ids.push(id);
            if ids.len() == MAX_MEMORY_CITATIONS {
                break;
            }
        }
    }
    ids
}

/// Source-only citations navigate to evidence without inventing a fact identifier.
pub fn memory_source_citations(text: &str) -> Vec<EpisodeId> {
    let mut ids = Vec::new();
    for tail in text.split("[source:").skip(1) {
        let Some((id, _)) = tail.split_once(']') else {
            continue;
        };
        if !id.starts_with("episode:")
            || id.len() <= "episode:".len()
            || id.len() > 512
            || id
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control() || "[()< >".contains(ch))
        {
            continue;
        }
        let id = EpisodeId(id.into());
        if !ids.contains(&id) {
            ids.push(id);
        }
        if ids.len() == MAX_MEMORY_CITATIONS {
            break;
        }
    }
    ids
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
}
