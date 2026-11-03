//! Explainable browsing links. Shared subjects are context, never inferred relationships.

use super::Fact;

pub const MAX_MEMORY_CONNECTIONS: usize = 24;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryConnectionReason {
    PreviousVersion,
    NewerVersion,
    SameSource,
    SharedSubjects(Vec<String>),
}

impl MemoryConnectionReason {
    pub fn label(&self) -> String {
        match self {
            Self::PreviousVersion => "Earlier version of this detail".into(),
            Self::NewerVersion => "Updated version of this detail".into(),
            Self::SameSource => "From the same original message".into(),
            Self::SharedSubjects(names) => format!("Also mentions {}", names.join(", ")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConnection {
    pub fact: Fact,
    pub reason: MemoryConnectionReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryNeighborhood {
    pub connections: Vec<MemoryConnection>,
    /// The adapter or presentation bound omitted some matches.
    pub has_more: bool,
}

fn rank(reason: &MemoryConnectionReason) -> u8 {
    match reason {
        MemoryConnectionReason::NewerVersion => 0,
        MemoryConnectionReason::PreviousVersion => 1,
        MemoryConnectionReason::SameSource => 2,
        MemoryConnectionReason::SharedSubjects(_) => 3,
    }
}

/// Prefer explicit version/source links to co-occurrence. Stable tie-breaking
/// makes the same saved neighborhood render in the same order after reopening.
pub fn memory_neighborhood(anchor: &Fact, candidates: Vec<Fact>) -> MemoryNeighborhood {
    let mut seen = std::collections::HashSet::new();
    let mut connections = candidates
        .into_iter()
        .filter_map(|fact| {
            if fact.id == anchor.id || !seen.insert(fact.id.clone()) {
                return None;
            }
            let reason = if anchor.replaces.as_ref() == Some(&fact.id) {
                MemoryConnectionReason::PreviousVersion
            } else if fact.replaces.as_ref() == Some(&anchor.id) {
                MemoryConnectionReason::NewerVersion
            } else if !fact.is_current() {
                return None;
            } else if anchor.source.is_some() && anchor.source == fact.source {
                MemoryConnectionReason::SameSource
            } else {
                let mut shared = anchor
                    .entities
                    .iter()
                    .filter(|name| {
                        !name.eq_ignore_ascii_case("user") && fact.entities.contains(name)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                shared.sort();
                shared.dedup();
                if shared.is_empty() {
                    return None;
                }
                MemoryConnectionReason::SharedSubjects(shared)
            };
            Some(MemoryConnection { fact, reason })
        })
        .collect::<Vec<_>>();
    connections.sort_by(|a, b| {
        rank(&a.reason)
            .cmp(&rank(&b.reason))
            .then(b.fact.created_at.cmp(&a.fact.created_at))
            .then(a.fact.id.0.cmp(&b.fact.id.0))
    });
    let has_more = connections.len() > MAX_MEMORY_CONNECTIONS;
    connections.truncate(MAX_MEMORY_CONNECTIONS);
    MemoryNeighborhood {
        connections,
        has_more,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FactId, Timestamp};

    fn fact(id: &str, names: &[&str]) -> Fact {
        Fact {
            id: FactId(id.into()),
            content: id.into(),
            entities: names.iter().map(|name| (*name).into()).collect(),
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
    fn connections_explain_real_links_without_inventing_relationships() {
        let mut anchor = fact("anchor", &["user", "Mira"]);
        anchor.replaces = Some(FactId("before".into()));
        let mut before = fact("before", &[]);
        before.expired_at = Some(Timestamp(2));
        let related = fact("related", &["Mira"]);
        let mut stopped = fact("stopped", &["Mira"]);
        stopped.stopped_at = Some(Timestamp(2));
        let result = memory_neighborhood(
            &anchor,
            vec![
                related.clone(),
                fact("unrelated", &["user"]),
                stopped,
                before,
                related,
            ],
        );
        assert_eq!(result.connections.len(), 2);
        assert_eq!(
            result.connections[0].reason,
            MemoryConnectionReason::PreviousVersion
        );
        assert_eq!(result.connections[1].reason.label(), "Also mentions Mira");
        assert!(!result.has_more);
    }

    #[test]
    fn same_source_and_new_versions_take_priority_and_the_neighborhood_is_bounded() {
        let mut anchor = fact("anchor", &["Project"]);
        anchor.source = Some(crate::EpisodeId("episode:one".into()));
        let mut newer = fact("newer", &[]);
        newer.replaces = Some(anchor.id.clone());
        let mut sibling = fact("sibling", &[]);
        sibling.source = anchor.source.clone();
        let mut candidates = (0..30)
            .map(|n| fact(&format!("related-{n}"), &["Project"]))
            .collect::<Vec<_>>();
        candidates.extend([sibling, newer]);
        let result = memory_neighborhood(&anchor, candidates);
        assert_eq!(result.connections.len(), MAX_MEMORY_CONNECTIONS);
        assert!(result.has_more);
        assert_eq!(
            result.connections[0].reason,
            MemoryConnectionReason::NewerVersion
        );
        assert_eq!(
            result.connections[1].reason.label(),
            "From the same original message"
        );
    }
}
