//! Original evidence stays distinct from model interpretations and personal facts.

use super::{Episode, EpisodeId, EpisodeKind, MemoryError, MemorySearch, Timestamp};

/// Origins are assigned by producers, never inferred from model output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceOrigin {
    UserStatement,
    AssistantOutput,
    DocumentClaim,
    LegacyUnknown,
}

/// This first source reader admits only explicitly private conversation evidence.
/// Unknown legacy permissions are never promoted to private access by migration.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceScope {
    Private,
    Unknown,
}

/// An exact span in one immutable source version. It does not assert current truth.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MemorySourceEvidence {
    pub citation: String,
    pub source_id: EpisodeId,
    pub version: u64,
    pub origin: MemorySourceOrigin,
    pub scope: MemorySourceScope,
    pub conversation: String,
    pub source_time: Timestamp,
    pub start: usize,
    pub end: usize,
    pub quote: String,
    /// Supporting facts have changed by the requested knowledge/validity dates.
    /// The original helper wording remains historical evidence, not current truth.
    pub support_changed: bool,
}

/// Preserve evidence from different search terms before applying the shared limit.
/// Versions remain distinct; repeated hits in one leg cannot receive extra votes.
pub fn fuse_memory_sources(
    mut legs: Vec<Vec<MemorySourceEvidence>>,
    limit: usize,
) -> Vec<MemorySourceEvidence> {
    // A smaller bounded leg is more selective; prefer it when ranks tie.
    legs.sort_by_key(Vec::len);
    super::ranked_fusion(legs, limit, |source| {
        (source.source_id.clone(), source.version)
    })
    .into_iter()
    .map(|(source, _)| source)
    .collect()
}

impl MemorySourceEvidence {
    /// Called only after storage has checked admission and current permissions.
    pub fn private_user(source: &Episode, query: &MemorySearch) -> Result<Self, MemoryError> {
        if !matches!(source.kind, EpisodeKind::UserMessage | EpisodeKind::Note) {
            return Err(MemoryError::InvalidInput(
                "not user-authored evidence".into(),
            ));
        }
        Self::private_admitted(source, query)
    }

    /// Storage must verify explicit permission and complete helper dependencies first.
    pub fn private_admitted(source: &Episode, query: &MemorySearch) -> Result<Self, MemoryError> {
        let origin = match source.kind {
            EpisodeKind::UserMessage | EpisodeKind::Note => MemorySourceOrigin::UserStatement,
            EpisodeKind::AgentMessage => MemorySourceOrigin::AssistantOutput,
            EpisodeKind::ToolCall | EpisodeKind::ToolResult => {
                return Err(MemoryError::InvalidInput(
                    "tool records need their own permission contract".into(),
                ));
            }
        };
        let terms = query.terms();
        let matched = source
            .content
            .char_indices()
            .find_map(|(offset, _)| {
                terms
                    .iter()
                    .any(|term| {
                        source.content[offset..]
                            .get(..term.len())
                            .is_some_and(|text| text.to_lowercase() == *term)
                    })
                    .then_some(offset)
            })
            .unwrap_or(0);
        let mut start = matched.saturating_sub(256);
        while !source.content.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = (start + 1536).min(source.content.len());
        while !source.content.is_char_boundary(end) {
            end -= 1;
        }
        Ok(Self {
            citation: format!("source:{}", source.id.0),
            source_id: source.id.clone(),
            version: 1,
            origin,
            scope: MemorySourceScope::Private,
            conversation: source.session_id.clone(),
            source_time: source.occurred_at,
            start,
            end,
            quote: source.content[start..end].to_owned(),
            support_changed: false,
        })
    }
}

/// Local review metadata. Unknown legacy values stay unknown; review does not grant recall access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceMetadata {
    pub attribution: Option<super::MemorySourceAttribution>,
    pub admitted_at: Option<Timestamp>,
    pub origin: MemorySourceOrigin,
    pub scope: MemorySourceScope,
    pub version: Option<u64>,
    pub revoked_at: Option<Timestamp>,
    pub processing: Option<super::MemoryJobState>,
    pub detail_count: Option<usize>,
}

impl Default for MemorySourceMetadata {
    fn default() -> Self {
        Self {
            attribution: None,
            admitted_at: None,
            origin: MemorySourceOrigin::LegacyUnknown,
            scope: MemorySourceScope::Unknown,
            version: None,
            revoked_at: None,
            processing: None,
            detail_count: None,
        }
    }
}

impl MemorySourceMetadata {
    pub fn author_label(&self) -> String {
        if self.origin == MemorySourceOrigin::AssistantOutput
            && let Some(super::MemorySourceAttribution {
                author:
                    super::MemorySourceAuthor::Assistant {
                        speaker: Some(speaker),
                    },
                ..
            }) = &self.attribution
        {
            return format!("A reply from {speaker}");
        }
        self.origin_label().into()
    }

    pub fn origin_label(&self) -> &'static str {
        match self.origin {
            MemorySourceOrigin::UserStatement => "A statement from you",
            MemorySourceOrigin::AssistantOutput => "A reply from a helper",
            MemorySourceOrigin::DocumentClaim => "A claim from a document",
            MemorySourceOrigin::LegacyUnknown => "Origin not recorded",
        }
    }

    pub fn status_label(&self) -> &'static str {
        if self.revoked_at.is_some() {
            return "Stopped using · retained for review";
        }
        match self.processing {
            Some(super::MemoryJobState::Pending) => "Evidence saved · waiting to be checked",
            Some(super::MemoryJobState::Processing) => "Evidence saved · checking details",
            Some(super::MemoryJobState::Complete) => "Evidence saved · check finished",
            Some(super::MemoryJobState::Failed) => "Evidence saved · check needs attention",
            Some(super::MemoryJobState::Cancelled) => "Evidence saved · checking stopped",
            None => "Saved record",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceRecord {
    pub episode: Episode,
    pub metadata: MemorySourceMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceSummary {
    pub id: EpisodeId,
    pub kind: EpisodeKind,
    pub occurred_at: Timestamp,
    pub excerpt: String,
    pub shortened: bool,
    pub metadata: MemorySourceMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceCursor {
    pub occurred_at: Timestamp,
    pub id: EpisodeId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceQuery {
    /// Exact saved fact whose original and repeated evidence selects supporting records.
    pub fact: Option<super::FactId>,
    /// Literal case-insensitive match across the original content, not only its excerpt.
    pub search: String,
    pub include_stopped: bool,
    pub before: Option<MemorySourceCursor>,
    pub limit: usize,
}

impl Default for MemorySourceQuery {
    fn default() -> Self {
        Self {
            fact: None,
            search: String::new(),
            include_stopped: false,
            before: None,
            limit: 20,
        }
    }
}

impl MemorySourceQuery {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.search.len() > 512
            || self
                .fact
                .as_ref()
                .is_some_and(|id| id.0.trim().is_empty() || id.0.len() > 512)
            || self.limit == 0
            || self.limit > super::MAX_MEMORY_PAGE
            || self
                .before
                .as_ref()
                .is_some_and(|cursor| cursor.id.0.trim().is_empty() || cursor.id.0.len() > 512)
        {
            return Err(MemoryError::InvalidInput(
                "invalid source browser request".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySourcePage {
    pub sources: Vec<MemorySourceSummary>,
    pub next: Option<MemorySourceCursor>,
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn admitted_helper_spans_remain_exact_and_distinct_from_user_statements() {
        let mut source = Episode {
            id: EpisodeId("episode:helper".into()),
            session_id: "chat".into(),
            kind: EpisodeKind::AgentMessage,
            content: "茶".repeat(900),
            occurred_at: Timestamp(1),
        };
        let query = MemorySearch::default();
        assert!(MemorySourceEvidence::private_user(&source, &query).is_err());
        let evidence = MemorySourceEvidence::private_admitted(&source, &query).unwrap();
        assert_eq!(evidence.origin, MemorySourceOrigin::AssistantOutput);
        assert_eq!(evidence.quote, source.content[evidence.start..evidence.end]);
        assert!(evidence.quote.len() <= 1536);
        source.kind = EpisodeKind::ToolResult;
        assert!(MemorySourceEvidence::private_admitted(&source, &query).is_err());
    }
    #[test]
    fn unknown_origin_and_checking_are_not_presented_as_personal_facts() {
        let mut metadata = MemorySourceMetadata::default();
        assert_eq!(metadata.origin_label(), "Origin not recorded");
        assert_eq!(metadata.detail_count, None);
        metadata.processing = Some(super::super::MemoryJobState::Complete);
        assert_eq!(metadata.status_label(), "Evidence saved · check finished");
        metadata.revoked_at = Some(Timestamp(1));
        assert_eq!(
            metadata.status_label(),
            "Stopped using · retained for review"
        );
    }

    #[test]
    fn named_helpers_remain_distinct_from_user_and_unknown_authorship() {
        let mut metadata = MemorySourceMetadata {
            origin: MemorySourceOrigin::AssistantOutput,
            attribution: Some(super::super::MemorySourceAttribution {
                producer_key: "reply".into(),
                author: super::super::MemorySourceAuthor::Assistant {
                    speaker: Some("Priya".into()),
                },
                provenance: super::super::MemorySourceProvenance::Team {
                    run_id: "run".into(),
                    member_id: "priya".into(),
                },
            }),
            ..Default::default()
        };
        assert_eq!(metadata.author_label(), "A reply from Priya");
        metadata.origin = MemorySourceOrigin::LegacyUnknown;
        assert_eq!(metadata.author_label(), "Origin not recorded");
        metadata.origin = MemorySourceOrigin::UserStatement;
        assert_eq!(metadata.author_label(), "A statement from you");
    }

    #[test]
    fn source_browser_rejects_unbounded_requests() {
        assert!(MemorySourceQuery::default().validate().is_ok());
        for query in [
            MemorySourceQuery {
                fact: Some(super::super::FactId(String::new())),
                ..Default::default()
            },
            MemorySourceQuery {
                fact: Some(super::super::FactId("f".repeat(513))),
                ..Default::default()
            },
            MemorySourceQuery {
                limit: 0,
                ..Default::default()
            },
            MemorySourceQuery {
                limit: 51,
                ..Default::default()
            },
            MemorySourceQuery {
                search: "茶".repeat(171),
                ..Default::default()
            },
            MemorySourceQuery {
                before: Some(MemorySourceCursor {
                    occurred_at: Timestamp(0),
                    id: EpisodeId(String::new()),
                }),
                ..Default::default()
            },
        ] {
            assert!(query.validate().is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_fusion_keeps_selective_terms_versions_and_deterministic_limits() {
        let source = |id: &str| {
            MemorySourceEvidence::private_user(
                &Episode {
                    id: EpisodeId(id.into()),
                    session_id: "fixture".into(),
                    kind: EpisodeKind::UserMessage,
                    occurred_at: Timestamp(1),
                    content: id.into(),
                },
                &MemorySearch {
                    text: id.into(),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let answer = source("answer");
        let question = source("question");
        let noise = source("noise");
        let legs = vec![
            vec![question.clone(), noise, source("other")],
            vec![question.clone(), answer.clone()],
        ];
        assert_eq!(
            fuse_memory_sources(legs.clone(), 2),
            vec![question, answer.clone()]
        );
        assert_eq!(fuse_memory_sources(legs, 0), Vec::new());
        let mut revised = answer.clone();
        revised.version = 2;
        assert_eq!(
            fuse_memory_sources(
                vec![vec![answer.clone(), answer.clone(), revised.clone()]],
                8
            ),
            vec![answer, revised]
        );
    }

    #[test]
    fn source_spans_preserve_unicode_and_do_not_relabel_assistant_output() {
        let mut episode = Episode {
            id: EpisodeId("episode:span".into()),
            session_id: "conversation".into(),
            kind: EpisodeKind::UserMessage,
            occurred_at: Timestamp(1),
            content: format!(
                "{} The omitted code is cedar. {}",
                "茶".repeat(900),
                "é".repeat(900)
            ),
        };
        let query = MemorySearch {
            text: "cedar".into(),
            ..Default::default()
        };
        let span = MemorySourceEvidence::private_user(&episode, &query).unwrap();
        assert_eq!(span.quote, episode.content[span.start..span.end]);
        assert!(span.quote.contains("cedar"));
        assert!(span.quote.len() <= 1536);
        episode.kind = EpisodeKind::AgentMessage;
        assert!(MemorySourceEvidence::private_user(&episode, &query).is_err());
    }
}
