//! Bounded evidence for model requests. Retrieval ranks are not confidence scores.

use super::{Episode, EpisodeKind, Fact, MemoryError, RecallQuery, Timestamp};

pub const MAX_MEMORY_CONTEXT_BYTES: usize = 16 * 1024;
pub const MAX_MEMORY_CONTEXT_ITEMS: usize = 8;
pub const MAX_MEMORY_PRIMARY_ITEMS: usize = 6;
pub const MAX_MEMORY_SEARCHES: usize = 4;
const MAX_BACKGROUND_ITEMS: usize = 8;
const MAX_BACKGROUND_BYTES: usize = 4 * 1024;
const MAX_RELATED_ITEMS: usize = MAX_MEMORY_CONTEXT_ITEMS - MAX_MEMORY_PRIMARY_ITEMS;
const MAX_RELATED_ENTITIES: usize = 2;
const MAX_RELATED_CANDIDATES: usize = 8;

/// Explicit temporal filters are milliseconds, never dates inferred by storage.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySearch {
    pub text: String,
    pub entity: Option<String>,
    pub valid_at: Option<Timestamp>,
    pub known_at: Option<Timestamp>,
}
impl MemorySearch {
    pub fn validate(&self) -> Result<(), MemoryError> {
        let mut query = RecallQuery::recent(MAX_MEMORY_CONTEXT_ITEMS);
        query.text = Some(self.text.clone());
        query.entity.clone_from(&self.entity);
        query.validate()
    }

    /// Search separate terms so a natural-language question is not an AND of every
    /// function word. No recency fallback: unrelated questions must abstain.
    pub fn terms(&self) -> Vec<String> {
        const STOP: &[&str] = &[
            "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "do",
            "does", "for", "from", "had", "has", "have", "how", "i", "in", "is", "it", "me", "my",
            "of", "on", "or", "our", "please", "s", "should", "some", "that", "the", "their",
            "them", "there", "these", "they", "this", "to", "us", "was", "we", "were", "what",
            "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
        ];
        let mut terms = Vec::new();
        for word in self.text.split(|c: char| !c.is_alphanumeric()) {
            let word = word.to_lowercase();
            if word.len() < 2
                || word.len() > 128
                || STOP.contains(&word.as_str())
                || terms.contains(&word)
            {
                continue;
            }
            terms.push(word);
            if terms.len() == 8 {
                break;
            }
        }
        terms
    }

    pub fn query(&self, text: String, now: Timestamp) -> RecallQuery {
        let mut query = RecallQuery::recent(24);
        query.text = Some(text);
        query.entity.clone_from(&self.entity);
        query.valid_at = Some(self.valid_at.or(self.known_at).unwrap_or(now));
        query.known_at = self.known_at;
        query.include_expired = self.valid_at.is_some() || self.known_at.is_some();
        query
    }

    /// A small current working set accompanies word-based retrieval. Never mix
    /// current beliefs into an explicitly historical lookup.
    pub fn background_query(&self, now: Timestamp) -> Option<RecallQuery> {
        if self.valid_at.is_some() || self.known_at.is_some() {
            return None;
        }
        let mut query = self.query(String::new(), now);
        query.text = None;
        query.limit = MAX_BACKGROUND_ITEMS;
        Some(query)
    }

    fn related_entities(&self, seeds: &[Fact], now: Timestamp) -> Vec<String> {
        let mut names = Vec::new();
        for name in seeds
            .iter()
            .take(MAX_MEMORY_PRIMARY_ITEMS)
            .filter(|fact| self.accepts(fact, now))
            .flat_map(|fact| fact.entities.iter().take(16))
        {
            // The common self label would turn one match into broad personal-history recall.
            if name.trim().is_empty()
                || name.len() > 256
                || name.eq_ignore_ascii_case("user")
                || self
                    .entity
                    .as_ref()
                    .is_some_and(|required| required != name)
                || names.contains(name)
            {
                continue;
            }
            names.push(name.clone());
            if names.len() == MAX_RELATED_ENTITIES {
                break;
            }
        }
        names
    }

    /// One bounded hop from direct matches, with the original time and entity scope.
    pub fn related_queries(&self, seeds: &[Fact], now: Timestamp) -> Vec<RecallQuery> {
        self.related_entities(seeds, now)
            .into_iter()
            .map(|name| {
                let mut query = self.query(String::new(), now);
                query.text = None;
                query.entity = Some(name);
                query.limit = MAX_RELATED_CANDIDATES;
                query
            })
            .collect()
    }

    /// Prefer source siblings that connect a matched entity to another entity.
    /// Co-occurrence supplies context, never proof of an inferred relationship.
    pub fn related_facts(
        &self,
        seeds: &[Fact],
        candidates: Vec<Fact>,
        now: Timestamp,
    ) -> Vec<Fact> {
        let names = self.related_entities(seeds, now);
        let mut seen = std::collections::HashSet::new();
        let mut selected = candidates
            .into_iter()
            .filter_map(|fact| {
                let anchor = names.iter().position(|name| fact.entities.contains(name))?;
                if !self.accepts(&fact, now)
                    || seeds.iter().any(|seed| seed.id == fact.id)
                    || !seen.insert(fact.id.clone())
                    || fact.source.is_none()
                    || !fact
                        .entities
                        .iter()
                        .any(|name| !name.trim().is_empty() && name != &names[anchor])
                {
                    return None;
                }
                let sibling = seeds.iter().any(|seed| seed.source == fact.source);
                Some((!sibling, anchor, fact))
            })
            .collect::<Vec<_>>();
        selected.sort_by_key(|(not_sibling, anchor, _)| (*not_sibling, *anchor));
        selected
            .into_iter()
            .take(MAX_RELATED_ITEMS)
            .map(|(_, _, fact)| self.as_known(fact))
            .collect()
    }

    pub fn accepts(&self, fact: &Fact, now: Timestamp) -> bool {
        let valid = self.valid_at.or(self.known_at).unwrap_or(now);
        let known = self.known_at.unwrap_or(now);
        let historical = self.valid_at.is_some() || self.known_at.is_some();
        fact.stopped_at.is_none()
            && (historical || fact.is_current())
            && fact.valid_at.is_none_or(|at| at <= valid)
            && (fact.invalid_at.is_none_or(|at| at > valid)
                || self
                    .known_at
                    .is_some_and(|at| fact.expired_at.is_some_and(|end| end > at)))
            && fact.created_at <= known
            && self
                .known_at
                .is_none_or(|at| fact.expired_at.is_none_or(|end| end > at))
            && self
                .entity
                .as_ref()
                .is_none_or(|name| fact.entities.contains(name))
    }

    /// Later correction metadata is not evidence available at an earlier known time.
    pub fn as_known(&self, mut fact: Fact) -> Fact {
        if self
            .known_at
            .is_some_and(|at| fact.expired_at.is_some_and(|end| end > at))
        {
            fact.invalid_at = None;
            fact.expired_at = None;
        }
        fact
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MemoryEvidence {
    pub citation: String,
    pub fact_id: String,
    pub source_id: String,
    pub conversation: String,
    pub statement: String,
    pub quote: String,
    pub interpretation: bool,
    pub source_time: Timestamp,
    pub learned_at: Timestamp,
    pub valid_at: Option<Timestamp>,
    pub invalid_at: Option<Timestamp>,
    pub superseded_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct MemoryContext {
    pub memories: Vec<MemoryEvidence>,
    pub sources: Vec<super::MemorySourceEvidence>,
    /// Recent current details, independently of query wording; not relevance-ranked
    /// or a complete personal profile. They share the total serialized context cap.
    pub background: Vec<MemoryEvidence>,
    pub notice: Option<String>,
}
impl MemoryContext {
    pub fn unavailable() -> Self {
        Self {
            memories: vec![],
            sources: vec![],
            background: vec![],
            notice: Some(
                "Saved details could not be checked. Do not invent remembered facts.".into(),
            ),
        }
    }
    pub fn push_source(&mut self, source: super::MemorySourceEvidence) -> bool {
        if self.sources.len() >= 4
            || self
                .sources
                .iter()
                .any(|item| item.source_id == source.source_id)
        {
            return false;
        }
        self.sources.push(source);
        if serde_json::to_vec(self).is_ok_and(|data| data.len() <= MAX_MEMORY_CONTEXT_BYTES) {
            true
        } else {
            self.sources.pop();
            false
        }
    }

    /// Use the same source grounding as direct evidence, with an additional small
    /// budget so always-visible context cannot crowd out targeted retrieval.
    pub fn push_background(&mut self, fact: &Fact, source: &Episode) -> bool {
        if !fact.is_current()
            || self.background.len() >= MAX_BACKGROUND_ITEMS
            || self
                .memories
                .iter()
                .chain(&self.background)
                .any(|item| item.fact_id == fact.id.0)
        {
            return false;
        }
        let mut grounded = Self::default();
        if !grounded.push(fact, source) {
            return false;
        }
        self.background.extend(grounded.memories);
        if serde_json::to_vec(&self.background).is_ok_and(|data| data.len() <= MAX_BACKGROUND_BYTES)
            && serde_json::to_vec(self).is_ok_and(|data| data.len() <= MAX_MEMORY_CONTEXT_BYTES)
        {
            true
        } else {
            self.background.pop();
            false
        }
    }

    /// Reject unsupported or oversized records; never truncate a quote into a
    /// different claim. The serialized evidence, including metadata, shares one cap.
    pub fn push(&mut self, fact: &Fact, source: &Episode) -> bool {
        if fact.stopped_at.is_some()
            || self.memories.len() >= MAX_MEMORY_CONTEXT_ITEMS
            || fact.source.as_ref() != Some(&source.id)
            || !matches!(source.kind, EpisodeKind::UserMessage | EpisodeKind::Note)
            || self.memories.iter().any(|item| item.fact_id == fact.id.0)
        {
            return false;
        }
        let quote = fact
            .extraction
            .as_ref()
            .map_or(fact.content.as_str(), |extraction| {
                extraction.evidence.as_str()
            });
        if quote.trim().is_empty()
            || !source.content.contains(quote)
            || fact.content.len() > 2048
            || quote.len() > 2048
            || fact.id.0.len() > 512
            || source.id.0.len() > 512
            || source.session_id.len() > 512
        {
            return false;
        }
        self.memories.push(MemoryEvidence {
            citation: format!("memory:{}", fact.id.0),
            fact_id: fact.id.0.clone(),
            source_id: source.id.0.clone(),
            conversation: source.session_id.clone(),
            statement: fact.content.clone(),
            quote: quote.into(),
            interpretation: fact.extraction.is_some(),
            source_time: source.occurred_at,
            learned_at: fact.created_at,
            valid_at: fact.valid_at,
            invalid_at: fact.invalid_at,
            superseded_at: fact.expired_at,
        });
        if serde_json::to_vec(self).is_ok_and(|data| data.len() <= MAX_MEMORY_CONTEXT_BYTES) {
            true
        } else {
            self.memories.pop();
            false
        }
    }
}

/// A per-turn reader. Implementations enforce the current remembering setting on
/// every request, including tool requests made after a pause.
#[async_trait::async_trait]
pub trait MemoryReader: Send + Sync {
    /// Automatic turn context may include a bounded recent working set.
    async fn search(&self, query: MemorySearch) -> Result<MemoryContext, MemoryError>;

    /// Focused lookups omit unrelated recent background details.
    /// Results preserve query order. Implementations must bound total work and
    /// recheck permissions after the whole batch, including already finished reads.
    /// Readers without that guarantee explicitly decline multi-query requests.
    async fn search_batch(
        &self,
        mut queries: Vec<MemorySearch>,
    ) -> Result<Vec<MemoryContext>, MemoryError> {
        validate_memory_searches(&queries)?;
        if queries.len() != 1 {
            return Err(MemoryError::Backend(
                "Multiple saved-detail searches are unavailable.".into(),
            ));
        }
        let mut context = self.search(queries.remove(0)).await?;
        context.background.clear();
        Ok(vec![context])
    }
}

pub fn validate_memory_searches(queries: &[MemorySearch]) -> Result<(), MemoryError> {
    if queries.is_empty() || queries.len() > MAX_MEMORY_SEARCHES {
        return Err(MemoryError::InvalidInput(
            "Use one to four saved-detail searches.".into(),
        ));
    }
    queries.iter().try_for_each(MemorySearch::validate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EpisodeId, FactId, MemoryExtraction};
    #[test]
    fn search_batches_are_bounded_and_validate_every_query() {
        let query = MemorySearch {
            text: "tea".into(),
            ..Default::default()
        };
        assert!(validate_memory_searches(&[]).is_err());
        assert!(validate_memory_searches(&vec![query.clone(); 4]).is_ok());
        assert!(validate_memory_searches(&vec![query.clone(); 5]).is_err());
        let invalid = MemorySearch {
            text: "x".repeat(4097),
            ..query.clone()
        };
        assert!(validate_memory_searches(&[query, invalid]).is_err());
    }
    fn pair() -> (Fact, Episode) {
        let source = Episode {
            id: EpisodeId("episode:1".into()),
            session_id: "earlier-chat".into(),
            kind: EpisodeKind::UserMessage,
            content: "I prefer tea".into(),
            occurred_at: Timestamp(1),
        };
        let fact = Fact {
            id: FactId("fact:1".into()),
            content: "User prefers tea".into(),
            entities: vec!["User".into()],
            source: Some(source.id.clone()),
            replaces: None,
            extraction: Some(MemoryExtraction {
                model: "test".into(),
                evidence: source.content.clone(),
            }),
            created_at: Timestamp(10),
            valid_at: Some(Timestamp(2)),
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        };
        (fact, source)
    }
    #[test]
    fn queries_abstain_on_function_words_and_preserve_temporal_entity_filters() {
        let mut search = MemorySearch {
            text: "What do I prefer with tea? tea".into(),
            ..Default::default()
        };
        assert_eq!(search.terms(), vec!["prefer", "tea"]);
        assert_eq!(
            MemorySearch {
                text: "What do I?".into(),
                ..Default::default()
            }
            .terms(),
            Vec::<String>::new()
        );
        search.entity = Some("User".into());
        search.valid_at = Some(Timestamp(5));
        let query = search.query("tea".into(), Timestamp(20));
        assert!(query.include_expired);
        assert_eq!(query.valid_at, Some(Timestamp(5)));
        assert_eq!(query.entity.as_deref(), Some("User"));
        search.text = "x".repeat(4097);
        assert!(search.validate().is_err());
    }
    #[test]
    fn related_queries_keep_scope_and_do_not_expand_self_or_missing_matches() {
        let (mut seed, _) = pair();
        seed.entities = vec![
            "user".into(),
            "Mira".into(),
            "Mira".into(),
            "Nora".into(),
            "Cedar".into(),
        ];
        let mut search = MemorySearch {
            text: "breakfast".into(),
            valid_at: Some(Timestamp(5)),
            known_at: Some(Timestamp(20)),
            ..Default::default()
        };
        let queries = search.related_queries(&[seed.clone()], Timestamp(30));
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0].entity.as_deref(), Some("Mira"));
        assert_eq!(queries[1].entity.as_deref(), Some("Nora"));
        for query in queries {
            assert_eq!(query.text, None);
            assert_eq!(query.limit, 8);
            assert_eq!(query.valid_at, Some(Timestamp(5)));
            assert_eq!(query.known_at, Some(Timestamp(20)));
            assert!(query.include_expired);
            assert!(query.validate().is_ok());
        }
        search.entity = Some("Mira".into());
        assert_eq!(
            search.related_queries(&[seed.clone()], Timestamp(30)).len(),
            1
        );
        search.entity = Some("user".into());
        assert_eq!(
            search.related_queries(&[seed.clone()], Timestamp(30)),
            Vec::<RecallQuery>::new()
        );
        search.entity = None;
        assert_eq!(
            search.related_queries(&[], Timestamp(30)),
            Vec::<RecallQuery>::new()
        );
        seed.stopped_at = Some(Timestamp(25));
        assert_eq!(
            search.related_queries(&[seed], Timestamp(30)),
            Vec::<RecallQuery>::new()
        );
    }

    #[test]
    fn related_context_is_bounded_source_preferred_and_temporally_valid() {
        let (mut seed, _) = pair();
        seed.entities = vec!["Mira".into()];
        let mut relationship = seed.clone();
        relationship.id.0 = "fact:relationship".into();
        relationship.entities.push("user".into());
        let mut other_source = relationship.clone();
        other_source.id.0 = "fact:other-source".into();
        other_source.source = Some(EpisodeId("episode:other".into()));
        let mut candidates = vec![
            other_source.clone(),
            seed.clone(),
            relationship.clone(),
            relationship.clone(),
        ];
        let mut extra = other_source.clone();
        extra.id.0 = "fact:third-context".into();
        candidates.push(extra);
        for id in [
            "withdrawn",
            "future",
            "expired",
            "unrelated",
            "no-source",
            "single-entity",
        ] {
            let mut fact = relationship.clone();
            fact.id.0 = id.into();
            match id {
                "withdrawn" => fact.stopped_at = Some(Timestamp(20)),
                "future" => fact.valid_at = Some(Timestamp(100)),
                "expired" => {
                    fact.invalid_at = Some(Timestamp(20));
                    fact.expired_at = Some(Timestamp(25));
                }
                "unrelated" => fact.entities = vec!["Nora".into(), "user".into()],
                "no-source" => fact.source = None,
                "single-entity" => {
                    fact.entities = vec!["Mira".into(), "Mira".into(), String::new()];
                }
                _ => unreachable!(),
            }
            candidates.push(fact);
        }
        let search = MemorySearch::default();
        assert_eq!(
            search.related_facts(&[seed.clone()], candidates, Timestamp(50)),
            vec![relationship.clone(), other_source]
        );
        assert_eq!(
            search.related_facts(&[], vec![relationship.clone()], Timestamp(50)),
            Vec::<Fact>::new()
        );
        relationship.invalid_at = Some(Timestamp(30));
        relationship.expired_at = Some(Timestamp(40));
        let historical = MemorySearch {
            valid_at: Some(Timestamp(35)),
            known_at: Some(Timestamp(25)),
            ..Default::default()
        };
        let earlier = historical.related_facts(&[seed], vec![relationship], Timestamp(50));
        assert_eq!(earlier.len(), 1);
        assert_eq!(earlier[0].invalid_at, None);
        assert_eq!(earlier[0].expired_at, None);
    }

    #[test]
    fn current_future_historical_and_known_time_are_distinct() {
        let (mut fact, _) = pair();
        let mut query = MemorySearch::default();
        assert!(!query.accepts(&fact, Timestamp(1)));
        assert!(query.accepts(&fact, Timestamp(20)));
        fact.invalid_at = Some(Timestamp(30));
        fact.expired_at = Some(Timestamp(40));
        assert!(!query.accepts(&fact, Timestamp(50)));
        query.valid_at = Some(Timestamp(20));
        assert!(query.accepts(&fact, Timestamp(50)));
        query.known_at = Some(Timestamp(5));
        assert!(!query.accepts(&fact, Timestamp(50)));
        query.known_at = Some(Timestamp(45));
        assert!(!query.accepts(&fact, Timestamp(50)));
        query.known_at = Some(Timestamp(25));
        assert!(query.accepts(&fact, Timestamp(50)));
        query.valid_at = Some(Timestamp(35));
        assert!(
            query.accepts(&fact, Timestamp(50)),
            "the correction was not known yet"
        );
        let historical = query.as_known(fact);
        assert!(historical.is_current());
    }

    #[test]
    fn withdrawn_evidence_cannot_reappear_at_an_earlier_known_or_valid_time() {
        let (mut fact, source) = pair();
        fact.stopped_at = Some(Timestamp(50));
        let query = MemorySearch {
            text: "tea".into(),
            valid_at: Some(Timestamp(5)),
            known_at: Some(Timestamp(15)),
            ..Default::default()
        };
        assert!(!query.accepts(&fact, Timestamp(100)));
        assert!(!query.as_known(fact.clone()).is_current());
        assert!(!MemoryContext::default().push(&fact, &source));
    }
    #[test]
    fn background_is_current_grounded_deduplicated_and_bounded() {
        let (mut fact, mut source) = pair();
        let mut context = MemoryContext::default();
        assert!(context.push_background(&fact, &source));
        assert!(!context.push_background(&fact, &source));
        assert_eq!(context.memories, Vec::new());
        assert_eq!(context.background[0].citation, "memory:fact:1");
        context.background.clear();
        assert!(context.push(&fact, &source));
        assert!(!context.push_background(&fact, &source));
        let mut context = MemoryContext::default();
        fact.expired_at = Some(Timestamp(30));
        assert!(!context.push_background(&fact, &source));
        fact.expired_at = None;
        fact.stopped_at = Some(Timestamp(30));
        assert!(!context.push_background(&fact, &source));
        fact.stopped_at = None;
        source.kind = EpisodeKind::AgentMessage;
        assert!(!context.push_background(&fact, &source));
        source.kind = EpisodeKind::UserMessage;
        for index in 0..50 {
            fact.id.0 = format!("fact:{index}");
            context.push_background(&fact, &source);
        }
        assert_eq!(context.background.len(), MAX_BACKGROUND_ITEMS);
        assert!(serde_json::to_vec(&context.background).unwrap().len() <= MAX_BACKGROUND_BYTES);
        assert!(serde_json::to_vec(&context).unwrap().len() <= MAX_MEMORY_CONTEXT_BYTES);
        let mut context = MemoryContext::default();
        fact.content = "\\".repeat(2048);
        source.content = fact.content.clone();
        fact.extraction = None;
        assert!(!context.push_background(&fact, &source));
        assert_eq!(context.background, Vec::new());
    }

    #[test]
    fn background_query_preserves_entity_scope_and_omits_historical_reads() {
        let mut search = MemorySearch {
            text: "different words".into(),
            entity: Some("Mira".into()),
            ..Default::default()
        };
        let query = search.background_query(Timestamp(80)).unwrap();
        assert_eq!(query.entity.as_deref(), Some("Mira"));
        assert_eq!(query.valid_at, Some(Timestamp(80)));
        assert_eq!(query.limit, MAX_BACKGROUND_ITEMS);
        assert!(query.text.is_none());
        assert!(!query.include_expired);
        search.valid_at = Some(Timestamp(40));
        assert!(search.background_query(Timestamp(80)).is_none());
        search.valid_at = None;
        search.known_at = Some(Timestamp(40));
        assert!(search.background_query(Timestamp(80)).is_none());
    }

    #[test]
    fn context_requires_user_evidence_and_bounds_actual_json() {
        let (mut fact, mut source) = pair();
        let mut context = MemoryContext::default();
        assert!(context.push(&fact, &source));
        assert!(!context.push(&fact, &source));
        assert_eq!(context.memories[0].citation, "memory:fact:1");
        source.kind = EpisodeKind::AgentMessage;
        fact.id.0 = "fact:2".into();
        assert!(!context.push(&fact, &source));
        source.kind = EpisodeKind::UserMessage;
        source.content = "different source".into();
        assert!(!context.push(&fact, &source));
        source.content = "I prefer tea".into();
        for index in 2..50 {
            fact.id.0 = format!("fact:{index}");
            context.push(&fact, &source);
        }
        assert_eq!(context.memories.len(), MAX_MEMORY_CONTEXT_ITEMS);
        assert!(serde_json::to_vec(&context).unwrap().len() <= MAX_MEMORY_CONTEXT_BYTES);
        let mut context = MemoryContext::default();
        fact.content = "\\".repeat(2048);
        source.content = fact.content.clone();
        fact.extraction = None;
        for index in 0..20 {
            fact.id.0 = format!("fact:{index}");
            context.push(&fact, &source);
        }
        assert!(context.memories.len() < MAX_MEMORY_CONTEXT_ITEMS);
        assert!(serde_json::to_vec(&context).unwrap().len() <= MAX_MEMORY_CONTEXT_BYTES);
    }

    #[test]
    fn personal_statement_is_recalled_from_another_chat_and_chatter_is_not_saved() {
        use super::super::{MemoryInput, NewEpisode, parse_memory_candidates};

        let vegan = Episode {
            id: EpisodeId("episode:vegan".into()),
            session_id: "chat-a".into(),
            kind: EpisodeKind::UserMessage,
            content: "I am vegan".into(),
            occurred_at: Timestamp(10),
        };
        let saved = parse_memory_candidates(
            &vegan,
            r#"{"memories":[{"content":"User is vegan","evidence":"I am vegan","entities":["user"],"valid_at":null,"replaces":null}]}"#,
            &[],
        )
        .unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].evidence, "I am vegan");

        let weather = Episode {
            id: EpisodeId("episode:weather".into()),
            session_id: "chat-a".into(),
            kind: EpisodeKind::UserMessage,
            content: "The weather is lovely today".into(),
            occurred_at: Timestamp(11),
        };
        let chatter = parse_memory_candidates(&weather, r#"{"memories":[]}"#, &[]).unwrap();
        assert!(
            chatter.is_empty(),
            "a non-personal aside produces no lasting fact"
        );
        assert!(
            parse_memory_candidates(
                &weather,
                r#"{"memories":[{"content":"User likes rain","evidence":"I love rain","entities":["user"],"valid_at":null,"replaces":null}]}"#,
                &[],
            )
            .is_err(),
            "a quote that is not in the message is refused"
        );
        let agent = MemoryInput {
            key: "helper".into(),
            episode: NewEpisode {
                session_id: "chat-a".into(),
                kind: EpisodeKind::AgentMessage,
                content: "I am vegan".into(),
                occurred_at: Timestamp(12),
            },
        };
        assert!(agent.validate().is_err());

        let fact = Fact {
            id: FactId("fact:vegan".into()),
            content: saved[0].content.clone(),
            entities: saved[0].entities.clone(),
            source: Some(vegan.id.clone()),
            replaces: None,
            extraction: Some(MemoryExtraction {
                model: "test".into(),
                evidence: saved[0].evidence.clone(),
            }),
            created_at: Timestamp(20),
            valid_at: None,
            invalid_at: None,
            expired_at: None,
            stopped_at: None,
        };
        let later = MemorySearch {
            text: "Am I vegan?".into(),
            ..Default::default()
        };
        assert!(later.terms().iter().any(|term| term == "vegan"));
        assert!(later.accepts(&fact, Timestamp(30)));
        let mut context = MemoryContext::default();
        assert!(context.push(&fact, &vegan));
        assert_eq!(context.memories[0].quote, "I am vegan");
        assert_eq!(context.memories[0].conversation, "chat-a");
        assert_eq!(context.memories[0].citation, "memory:fact:vegan");
        assert_eq!(context.memories[0].source_id, "episode:vegan");

        let mut crowded = context.clone();
        let mut extra = fact.clone();
        let mut extra_source = vegan.clone();
        for index in 0..30 {
            extra.id.0 = format!("fact:extra-{index}");
            extra_source.id.0 = extra.id.0.clone();
            extra.source = Some(extra_source.id.clone());
            extra.content = format!("User is vegan note {index}");
            extra.extraction = Some(MemoryExtraction {
                model: "test".into(),
                evidence: vegan.content.clone(),
            });
            extra_source.content = vegan.content.clone();
            crowded.push(&extra, &extra_source);
        }
        assert!(crowded.memories.len() <= MAX_MEMORY_CONTEXT_ITEMS);
        assert!(serde_json::to_vec(&crowded).unwrap().len() <= MAX_MEMORY_CONTEXT_BYTES);

        let mut withdrawn = fact.clone();
        withdrawn.stopped_at = Some(Timestamp(40));
        assert!(!later.accepts(&withdrawn, Timestamp(50)));
        assert!(!MemoryContext::default().push(&withdrawn, &vegan));

        let corrected = parse_memory_candidates(
            &vegan,
            r#"{"memories":[{"content":"User is vegan","evidence":"I am vegan","entities":["user"],"valid_at":null,"replaces":"fact:vegan"}]}"#,
            &[FactId("fact:vegan".into())],
        )
        .unwrap();
        assert_eq!(
            corrected[0].replaces.as_ref().map(|id| id.0.as_str()),
            Some("fact:vegan")
        );
        let mut ended = fact.clone();
        ended.invalid_at = Some(Timestamp(25));
        ended.expired_at = Some(Timestamp(25));
        assert!(!later.accepts(&ended, Timestamp(30)));
        assert_eq!(fact.content, "User is vegan");
    }
}
