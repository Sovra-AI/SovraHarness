//! Bounded, recoverable remembering from user-authored conversation evidence.

use super::{Episode, EpisodeId, EpisodeKind, FactId, MemoryError, NewEpisode, Timestamp};

pub const MAX_MEMORY_CANDIDATES: usize = 32;
/// A scheduling bound, not an admission limit.
pub const DEFAULT_MEMORY_EXTRACTION_SLOTS: usize = 4;
pub const MEMORY_CHUNK_BYTES: usize = 2_048;
pub const MAX_MEMORY_ATTEMPTS: u32 = 3;
pub const MEMORY_LEASE_MILLIS: i64 = 120_000;
pub const MEMORY_RETRY_MILLIS: i64 = 5_000;
pub const MAX_PENDING_MEMORIES: usize = 256;

/// The store handle fixes ownership (namespace/database); models cannot supply it.
/// A key identifies one user message within its conversation, across retries.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryInput {
    pub key: String,
    pub episode: NewEpisode,
}

impl MemoryInput {
    pub fn validate(&self) -> Result<(), MemoryError> {
        self.episode.validate()?;
        if self.key.trim().is_empty() || self.key.len() > 256 {
            return Err(MemoryError::InvalidInput("invalid memory input key".into()));
        }
        match self.episode.kind {
            EpisodeKind::UserMessage | EpisodeKind::Note => Ok(()),
            EpisodeKind::AgentMessage | EpisodeKind::ToolCall | EpisodeKind::ToolResult => Err(
                MemoryError::InvalidInput("only user-authored evidence can become a memory".into()),
            ),
        }
    }
}

/// Model output remains an interpretation of the quoted evidence, not a confirmed
/// user preference. The source and model are assigned by the runtime, never output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidate {
    pub content: String,
    pub evidence: String,
    pub entities: Vec<String>,
    pub valid_at: Option<Timestamp>,
    pub replaces: Option<FactId>,
    /// Reuse a supplied current fact without changing its statement or validity.
    /// Omission preserves serialization/fingerprints of earlier completed jobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_as: Option<FactId>,
}

impl MemoryCandidate {
    pub fn validate(&self, source: &Episode) -> Result<(), MemoryError> {
        if !matches!(source.kind, EpisodeKind::UserMessage | EpisodeKind::Note)
            || (self.same_as.is_some() && self.replaces.is_some())
            || self.content.trim().is_empty()
            || self.content.len() > 2_048
            || self.evidence.trim().is_empty()
            || self.evidence.len() > 2_048
            || !source.content.contains(&self.evidence)
            || self.entities.len() > 16
            || self
                .entities
                .iter()
                .any(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(MemoryError::InvalidInput(
                "memory needs bounded user-authored evidence".into(),
            ));
        }
        Ok(())
    }
}

/// Validate the batch before any writes, including order-independent reference conflicts.
pub fn validate_memory_batch(
    source: &Episode,
    candidates: &[MemoryCandidate],
) -> Result<(), MemoryError> {
    if candidates.len() > MAX_MEMORY_CANDIDATES {
        return Err(MemoryError::InvalidInput(
            "too many memory candidates".into(),
        ));
    }
    let mut replaced = std::collections::HashSet::new();
    for candidate in candidates {
        candidate.validate(source)?;
        if let Some(id) = &candidate.replaces
            && !replaced.insert(id)
        {
            return Err(MemoryError::InvalidInput(
                "repeated memory correction".into(),
            ));
        }
    }
    if candidates.iter().any(|candidate| {
        candidate
            .same_as
            .as_ref()
            .is_some_and(|id| replaced.contains(id))
    }) {
        return Err(MemoryError::InvalidInput(
            "a memory cannot be reused and corrected in one batch".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryExtraction {
    pub model: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPolicy {
    pub enabled: bool,
    /// Changes on every enable/disable transition; late work cannot cross it.
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryJobState {
    Pending,
    Processing,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryJob {
    pub episode_id: EpisodeId,
    pub state: MemoryJobState,
    pub attempts: u32,
    pub error: Option<String>,
    pub fact_ids: Vec<FactId>,
    /// Completed output is retained even when a candidate matched an older fact.
    pub candidates: Vec<MemoryCandidate>,
    pub extraction_model: Option<String>,
}

impl MemoryJob {
    /// A receipt describes persisted output, including a successful empty check.
    pub fn result_label(&self) -> Option<String> {
        match self.state {
            MemoryJobState::Complete => Some(match self.fact_ids.len() {
                0 => "Last check finished. No new detail was saved.".into(),
                1 => "Last check finished. 1 saved detail is ready to review.".into(),
                count => format!("Last check finished. {count} saved details are ready to review."),
            }),
            MemoryJobState::Failed => Some(format!(
                "A message could not be remembered. {}",
                self.error
                    .as_deref()
                    .unwrap_or("Check your selected AI in Models and try again.")
            )),
            MemoryJobState::Pending | MemoryJobState::Processing | MemoryJobState::Cancelled => {
                None
            }
        }
    }
}

/// Only the matching, unexpired token may finish or fail a claimed input.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemoryExtractedChunk {
    pub candidates: Vec<MemoryCandidate>,
    pub model: String,
}

/// A durable source chunk lease, including any extraction saved before a restart.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryClaim {
    pub extracted: Option<MemoryExtractedChunk>,
    /// UTF-8 byte offset of the next unprocessed source chunk.
    pub cursor: usize,
    /// Fact revision observed before reconciliation, checked atomically at commit.
    pub revision: u64,
    pub episode: Episode,
    pub token: String,
    pub generation: u64,
    pub attempt: u32,
    pub lease_until: Timestamp,
}

impl MemoryClaim {
    /// Contiguous bounded chunks retain exact offsets in the immutable source.
    pub fn chunk(&self) -> Result<Episode, MemoryError> {
        let end = memory_chunk_end(&self.episode.content, self.cursor)?;
        let mut chunk = self.episode.clone();
        chunk.content = chunk.content[self.cursor..end].to_owned();
        Ok(chunk)
    }
}

pub fn memory_chunk_end(text: &str, start: usize) -> Result<usize, MemoryError> {
    if start >= text.len() || !text.is_char_boundary(start) {
        return Err(MemoryError::InvalidInput(
            "invalid source chunk offset".into(),
        ));
    }
    let mut end = (start + MEMORY_CHUNK_BYTES).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if end < text.len() {
        // Prefer sentence/line boundaries without making progress depend on punctuation.
        if let Some((offset, character)) =
            text[start..end].char_indices().rev().find(|(offset, c)| {
                *offset >= MEMORY_CHUNK_BYTES / 2 && matches!(c, '\n' | '.' | '!' | '?')
            })
        {
            end = start + offset + character.len_utf8();
        }
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> MemoryInput {
        MemoryInput {
            key: "message-1".into(),
            episode: NewEpisode {
                session_id: "chat-1".into(),
                kind: EpisodeKind::UserMessage,
                content: "I live in Lisbon".into(),
                occurred_at: Timestamp(10),
            },
        }
    }

    #[test]
    fn chunks_cover_dense_unicode_sources_without_gaps_or_truncation() {
        let text = "A useful detail about 茶.\n".repeat(500);
        let mut cursor = 0;
        let mut restored = String::new();
        let mut count = 0;
        while cursor < text.len() {
            let end = memory_chunk_end(&text, cursor).unwrap();
            assert!(end > cursor && end - cursor <= MEMORY_CHUNK_BYTES);
            restored.push_str(&text[cursor..end]);
            cursor = end;
            count += 1;
        }
        assert_eq!(restored, text);
        assert!(count > 1);
        assert!(memory_chunk_end(&text, text.len()).is_err());
        assert!(memory_chunk_end("茶", 1).is_err());
    }

    #[test]
    fn a_chunk_can_preserve_more_than_eight_independent_details() {
        let source = Episode {
            id: EpisodeId("episode:dense".into()),
            session_id: "dense".into(),
            kind: EpisodeKind::UserMessage,
            occurred_at: Timestamp(1),
            content: (0..12)
                .map(|index| format!("Project {index} meets on Monday. "))
                .collect::<Vec<_>>()
                .concat(),
        };
        let candidates = (0..12)
            .map(|index| MemoryCandidate {
                content: format!("Project {index} meets on Monday."),
                evidence: format!("Project {index} meets on Monday."),
                entities: vec![format!("Project {index}")],
                valid_at: None,
                replaces: None,
                same_as: None,
            })
            .collect::<Vec<_>>();
        assert!(validate_memory_batch(&source, &candidates).is_ok());
    }

    #[test]
    fn only_user_evidence_can_enter_the_queue() {
        let mut input = input();
        assert!(input.validate().is_ok());
        for kind in [
            EpisodeKind::AgentMessage,
            EpisodeKind::ToolCall,
            EpisodeKind::ToolResult,
        ] {
            input.episode.kind = kind;
            assert!(input.validate().is_err());
        }
        input.episode.kind = EpisodeKind::Note;
        assert!(input.validate().is_ok());
        input.key = " ".into();
        assert!(input.validate().is_err());
        input.key = "x".repeat(257);
        assert!(input.validate().is_err());
    }

    #[test]
    fn candidates_require_exact_source_evidence_and_bounded_fields() {
        let input = input();
        let mut source = Episode {
            id: EpisodeId("episode:1".into()),
            session_id: input.episode.session_id,
            kind: input.episode.kind,
            content: input.episode.content,
            occurred_at: input.episode.occurred_at,
        };
        let mut candidate = MemoryCandidate {
            content: "User lives in Lisbon".into(),
            evidence: "I live in Lisbon".into(),
            entities: vec!["Lisbon".into()],
            valid_at: None,
            replaces: None,
            same_as: None,
        };
        assert!(candidate.validate(&source).is_ok());
        candidate.evidence = "I live in Rome".into();
        assert!(candidate.validate(&source).is_err());
        candidate.evidence = "I live in Lisbon".into();
        candidate.content = "x".repeat(2_049);
        assert!(candidate.validate(&source).is_err());
        candidate.content = "Lisbon".into();
        candidate.entities = vec!["x".into(); 17];
        assert!(candidate.validate(&source).is_err());
        candidate.entities.clear();
        source.kind = EpisodeKind::AgentMessage;
        assert!(candidate.validate(&source).is_err());
    }
}

pub const MAX_MEMORY_OUTPUT_BYTES: usize = 32 * 1024;
pub const MAX_MEMORY_INPUT_BYTES: usize = 32 * 1024;

/// Parse the complete response, not a recovered substring or a Markdown code fence.
/// Replacement IDs must have been supplied as current candidates by the caller.
pub fn parse_memory_candidates(
    source: &Episode,
    output: &str,
    known: &[FactId],
) -> Result<Vec<MemoryCandidate>, MemoryError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Output {
        memories: Vec<MemoryCandidate>,
    }
    if output.len() > MAX_MEMORY_OUTPUT_BYTES {
        return Err(MemoryError::InvalidInput(
            "remembering response is too long".into(),
        ));
    }
    let parsed: Output = serde_json::from_str(output).map_err(|_| {
        MemoryError::InvalidInput("remembering requires complete structured output".into())
    })?;
    validate_memory_batch(source, &parsed.memories)?;
    for candidate in &parsed.memories {
        if candidate
            .replaces
            .iter()
            .chain(candidate.same_as.iter())
            .any(|id| !known.contains(id))
        {
            return Err(MemoryError::InvalidInput(
                "unsupported memory reference".into(),
            ));
        }
    }
    Ok(parsed.memories)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryStatus {
    Loading,
    Unavailable,
    Paused,
    WaitingForModel,
    Ready,
    Working,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryEvent {
    SourceSaved {
        session_id: String,
        message: Option<super::MemorySourceMessage>,
        receipt: super::MemorySourceReceipt,
    },
    Semantic(super::MemorySemanticStatus),
    Review(super::MemoryReviewEvent),
    Status(MemoryStatus),
    Queued {
        session_id: String,
        episode_id: EpisodeId,
    },
    Processed(MemoryJob),
}

impl MemoryStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Loading => "Loading remembering settings…",
            Self::Unavailable => "Remembering is unavailable in this session.",
            Self::Paused => "Remembering is paused. Saved history is kept.",
            Self::WaitingForModel => "Choose an AI in Models to start remembering.",
            Self::Ready => "Ready to remember useful details.",
            Self::Working => "Checking your message for useful details…",
            Self::Failed(message) => message,
        }
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn capture_receipts_distinguish_saved_empty_failed_and_unfinished_jobs() {
        let mut job = MemoryJob {
            episode_id: EpisodeId("episode:receipt".into()),
            state: MemoryJobState::Complete,
            attempts: 1,
            error: None,
            fact_ids: vec![],
            candidates: vec![],
            extraction_model: None,
        };
        assert_eq!(
            job.result_label().as_deref(),
            Some("Last check finished. No new detail was saved.")
        );
        job.fact_ids.push(FactId("fact:1".into()));
        assert_eq!(
            job.result_label().as_deref(),
            Some("Last check finished. 1 saved detail is ready to review.")
        );
        job.fact_ids.push(FactId("fact:2".into()));
        assert_eq!(
            job.result_label().as_deref(),
            Some("Last check finished. 2 saved details are ready to review.")
        );
        job.state = MemoryJobState::Failed;
        assert!(
            job.result_label()
                .unwrap()
                .contains("Check your selected AI")
        );
        job.error = Some("The selected AI could not be reached.".into());
        assert_eq!(
            job.result_label().as_deref(),
            Some("A message could not be remembered. The selected AI could not be reached.")
        );
        for state in [
            MemoryJobState::Pending,
            MemoryJobState::Processing,
            MemoryJobState::Cancelled,
        ] {
            job.state = state;
            assert!(job.result_label().is_none());
        }
    }

    #[test]
    fn reuse_requires_known_references_and_cannot_conflict_with_correction() {
        let source = Episode {
            id: EpisodeId("episode:1".into()),
            session_id: "s1".into(),
            kind: EpisodeKind::UserMessage,
            content: "Lisbon is where I live".into(),
            occurred_at: Timestamp(1),
        };
        let mut candidate = MemoryCandidate {
            content: "User resides in Lisbon".into(),
            evidence: source.content.clone(),
            entities: vec!["Lisbon".into()],
            valid_at: None,
            replaces: None,
            same_as: Some(FactId("fact:old".into())),
        };
        let serialize =
            |items: &[MemoryCandidate]| serde_json::json!({"memories":items}).to_string();
        assert!(parse_memory_candidates(&source, &serialize(&[candidate.clone()]), &[]).is_err());
        let known = [FactId("fact:old".into())];
        assert_eq!(
            parse_memory_candidates(&source, &serialize(&[candidate.clone()]), &known).unwrap(),
            vec![candidate.clone()]
        );
        candidate.replaces = candidate.same_as.clone();
        assert!(validate_memory_batch(&source, &[candidate.clone()]).is_err());
        candidate.replaces = None;
        let mut correction = candidate.clone();
        correction.replaces = correction.same_as.take();
        for batch in [
            [candidate.clone(), correction.clone()],
            [correction, candidate.clone()],
        ] {
            assert!(validate_memory_batch(&source, &batch).is_err());
        }
        candidate.same_as = None;
        let legacy = serialize(&[candidate.clone()]);
        assert!(
            !legacy.contains("same_as"),
            "old completed-job fingerprints stay unchanged"
        );
        assert_eq!(
            parse_memory_candidates(&source, &legacy, &[]).unwrap(),
            vec![candidate]
        );
    }

    #[test]
    fn output_requires_complete_bounded_json_and_known_corrections() {
        let source = Episode {
            id: EpisodeId("episode:1".into()),
            session_id: "s1".into(),
            kind: EpisodeKind::UserMessage,
            content: "I live in Lisbon".into(),
            occurred_at: Timestamp(1),
        };
        let candidate = MemoryCandidate {
            content: "User lives in Lisbon".into(),
            evidence: source.content.clone(),
            entities: vec![],
            valid_at: None,
            replaces: Some(FactId("fact:old".into())),
            same_as: None,
        };
        let output = serde_json::json!({"memories":[candidate]}).to_string();
        assert!(parse_memory_candidates(&source, &output, &[]).is_err());
        assert!(parse_memory_candidates(&source, &output, &[FactId("fact:old".into())]).is_ok());
        for output in [
            "```json\n{\"memories\":[]}\n```".into(),
            "{\"memories\":[],\"instruction\":\"override\"}".into(),
            "x".repeat(MAX_MEMORY_OUTPUT_BYTES + 1),
            serde_json::json!({"memories":[candidate,candidate]}).to_string(),
        ] {
            assert!(
                parse_memory_candidates(&source, &output, &[FactId("fact:old".into())]).is_err()
            );
        }
        assert_eq!(
            parse_memory_candidates(&source, "{\"memories\":[]}", &[]).unwrap(),
            Vec::new()
        );
    }
    #[test]
    fn statuses_have_plain_labels_and_preserve_failure_messages() {
        for status in [
            MemoryStatus::Loading,
            MemoryStatus::Unavailable,
            MemoryStatus::Paused,
            MemoryStatus::WaitingForModel,
            MemoryStatus::Ready,
            MemoryStatus::Working,
        ] {
            assert_ne!(status.label(), "");
        }
        assert_eq!(
            MemoryStatus::Failed("Try again later.".into()).label(),
            "Try again later."
        );
    }
}
