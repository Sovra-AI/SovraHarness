//! Bound prior facts for extraction, prioritizing relevance over unrelated recency.

use super::{Fact, MemorySearch, Timestamp};

const MAX_KNOWN_FACTS: usize = 8;

pub fn consolidation_context(related: Vec<Fact>, recent: Vec<Fact>, now: Timestamp) -> Vec<Fact> {
    let current = MemorySearch::default();
    let mut seen = std::collections::HashSet::new();
    related
        .into_iter()
        .chain(recent)
        .filter(|fact| current.accepts(fact, now) && seen.insert(fact.id.clone()))
        .take(MAX_KNOWN_FACTS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FactId;

    fn fact(id: &str) -> Fact {
        Fact {
            id: FactId(id.into()),
            content: id.into(),
            entities: vec![],
            source: None,
            replaces: None,
            extraction: None,
            created_at: Timestamp(1),
            valid_at: None,
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        }
    }

    #[test]
    fn related_current_facts_win_and_history_cannot_reenter_the_extractor() {
        let relevant = fact("old relevant");
        let mut stopped = fact("stopped");
        stopped.stopped_at = Some(Timestamp(2));
        let mut expired = fact("expired");
        expired.expired_at = Some(Timestamp(2));
        let mut invalid = fact("invalid");
        invalid.invalid_at = Some(Timestamp(2));
        let mut future = fact("future");
        future.valid_at = Some(Timestamp(11));
        let mut unknown = fact("unknown");
        unknown.created_at = Timestamp(11);
        let recent = (0..10).map(|index| fact(&index.to_string())).collect();
        let result = consolidation_context(
            vec![
                stopped,
                expired,
                invalid,
                future,
                unknown,
                relevant.clone(),
                relevant.clone(),
            ],
            recent,
            Timestamp(10),
        );
        assert_eq!(result.len(), MAX_KNOWN_FACTS);
        assert_eq!(result[0], relevant);
        assert_eq!(result[1].id.0, "0");
        assert_eq!(
            consolidation_context(vec![], vec![], Timestamp(10)),
            Vec::<Fact>::new()
        );
    }
}
