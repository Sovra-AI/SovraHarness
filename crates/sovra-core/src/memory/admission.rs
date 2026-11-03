//! Producer-assigned identity and authorship; model output cannot grant source access.

use super::{
    EpisodeId, EpisodeKind, MemoryError, MemoryInput, MemorySourceOrigin, MemorySourceScope,
    NewEpisode,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MemorySourceAuthor {
    User,
    Assistant { speaker: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MemorySourceProvenance {
    Conversation,
    Team {
        run_id: String,
        member_id: String,
    },
    ConnectedMessage {
        connection_id: String,
        message_id: String,
    },
    ManualNote,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemorySourceAttribution {
    pub producer_key: String,
    pub author: MemorySourceAuthor,
    pub provenance: MemorySourceProvenance,
}

/// Stable producer identity within a conversation, independent of extraction.
/// The episode supplies the immutable wording, conversation identity and source time.
/// Storage assigns admission time. External document references use their own readers;
/// they must not be relabeled as conversation text to enter this contract.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySourceAdmission {
    /// Complete inputs seen by the helper, assigned by the runtime, never the model.
    /// None means unknown; Some(empty) explicitly certifies no saved inputs.
    pub dependencies: Option<MemorySourceDependencies>,
    /// A long-running capture supplies the policy generation observed at its start.
    /// Admission must reject it after pause/resume, even if remembering is on again.
    pub expected_generation: Option<u64>,
    pub key: String,
    pub version: u64,
    pub author: MemorySourceAuthor,
    pub scope: MemorySourceScope,
    pub provenance: MemorySourceProvenance,
    pub episode: NewEpisode,
}

impl MemorySourceAdmission {
    pub fn user(input: MemoryInput) -> Self {
        Self {
            dependencies: None,
            expected_generation: None,
            provenance: if input.episode.kind == EpisodeKind::Note {
                MemorySourceProvenance::ManualNote
            } else {
                MemorySourceProvenance::Conversation
            },
            key: input.key,
            version: 1,
            author: MemorySourceAuthor::User,
            scope: MemorySourceScope::Private,
            episode: input.episode,
        }
    }

    pub fn origin(&self) -> MemorySourceOrigin {
        match &self.author {
            MemorySourceAuthor::User => MemorySourceOrigin::UserStatement,
            MemorySourceAuthor::Assistant { .. } => MemorySourceOrigin::AssistantOutput,
        }
    }

    pub fn needs_extraction(&self) -> bool {
        matches!(self.author, MemorySourceAuthor::User)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        self.episode.validate()?;
        if let Some(dependencies) = &self.dependencies {
            dependencies.validate()?;
            if matches!(self.author, MemorySourceAuthor::User) {
                return Err(MemoryError::InvalidInput(
                    "user statements cannot certify helper dependencies".into(),
                ));
            }
        }
        let bounded = |text: &str| !text.trim().is_empty() && text.len() <= 256;
        if !bounded(&self.key)
            || self.version == 0
            || self.version > i64::MAX as u64
            || self.scope != MemorySourceScope::Private
        {
            return Err(MemoryError::InvalidInput(
                "source needs a stable identity and explicit private permission".into(),
            ));
        }
        let author_matches = match &self.author {
            MemorySourceAuthor::User => matches!(
                self.episode.kind,
                EpisodeKind::UserMessage | EpisodeKind::Note
            ),
            MemorySourceAuthor::Assistant { speaker } => {
                self.episode.kind == EpisodeKind::AgentMessage
                    && speaker.as_deref().is_none_or(bounded)
            }
        };
        let provenance_matches = match &self.provenance {
            MemorySourceProvenance::Conversation => self.episode.kind != EpisodeKind::Note,
            MemorySourceProvenance::ManualNote => self.episode.kind == EpisodeKind::Note,
            MemorySourceProvenance::Team { run_id, member_id } => {
                bounded(run_id) && bounded(member_id) && self.episode.kind != EpisodeKind::Note
            }
            MemorySourceProvenance::ConnectedMessage {
                connection_id,
                message_id,
            } => {
                bounded(connection_id)
                    && bounded(message_id)
                    && self.episode.kind != EpisodeKind::Note
            }
        };
        if !author_matches || !provenance_matches {
            return Err(MemoryError::InvalidInput(
                "source authorship and provenance do not match its content kind".into(),
            ));
        }
        Ok(())
    }
}

/// Both direct inputs and the stored transitive closure have this shared bound.
pub const MAX_MEMORY_SOURCE_DEPENDENCIES: usize = 128;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemorySourceDependencies {
    pub sources: Vec<EpisodeId>,
    pub facts: Vec<super::FactId>,
}

impl MemorySourceDependencies {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.sources.len().saturating_add(self.facts.len()) > MAX_MEMORY_SOURCE_DEPENDENCIES
            || self
                .sources
                .iter()
                .map(|id| &id.0)
                .chain(self.facts.iter().map(|id| &id.0))
                .any(|id| id.trim().is_empty() || id.len() > 512)
        {
            return Err(MemoryError::InvalidInput(
                "invalid helper dependencies".into(),
            ));
        }
        Ok(())
    }
}

/// Returned only after evidence (and user extraction work, when applicable) persists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceReceipt {
    pub source_id: EpisodeId,
    pub version: u64,
    pub has_processing_job: bool,
}

/// Associates a durable receipt with the exact visible wording, never the latest
/// arbitrary message. Team messages also carry their AG-UI message identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySourceMessage {
    pub id: Option<String>,
    pub in_reply_to: Option<String>,
    pub role: crate::MessageRole,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Timestamp;
    #[test]
    fn dependency_bounds_cover_combined_inputs_and_user_authorship() {
        let mut dependencies = MemorySourceDependencies {
            sources: vec![EpisodeId("source".into()); MAX_MEMORY_SOURCE_DEPENDENCIES],
            facts: vec![],
        };
        assert!(dependencies.validate().is_ok());
        dependencies.facts.push(super::super::FactId("fact".into()));
        assert!(dependencies.validate().is_err());
        dependencies = MemorySourceDependencies::default();
        dependencies.sources.push(EpisodeId(" ".into()));
        assert!(dependencies.validate().is_err());
        dependencies.sources[0] = EpisodeId("x".repeat(513));
        assert!(dependencies.validate().is_err());
        let mut source = MemorySourceAdmission::user(MemoryInput {
            key: "input".into(),
            episode: NewEpisode {
                session_id: "chat".into(),
                kind: EpisodeKind::UserMessage,
                content: "User wording".into(),
                occurred_at: Timestamp(1),
            },
        });
        source.dependencies = Some(MemorySourceDependencies::default());
        assert!(source.validate().is_err());
        source.author = MemorySourceAuthor::Assistant { speaker: None };
        source.episode.kind = EpisodeKind::AgentMessage;
        assert!(source.validate().is_ok());
    }
    #[test]
    fn admission_never_promotes_assistant_output_or_unknown_permissions() {
        let mut source = MemorySourceAdmission::user(MemoryInput {
            key: "message-1".into(),
            episode: NewEpisode {
                session_id: "chat".into(),
                kind: EpisodeKind::UserMessage,
                content: "A quoted statement.".into(),
                occurred_at: Timestamp(1),
            },
        });
        assert!(source.validate().is_ok());
        assert!(source.needs_extraction());
        source.episode.kind = EpisodeKind::AgentMessage;
        assert!(source.validate().is_err());
        source.author = MemorySourceAuthor::Assistant { speaker: None };
        assert!(source.validate().is_ok());
        assert!(!source.needs_extraction());
        assert_eq!(source.origin(), MemorySourceOrigin::AssistantOutput);
        source.scope = MemorySourceScope::Unknown;
        assert!(source.validate().is_err());
        source.scope = MemorySourceScope::Private;
        source.version = 0;
        assert!(source.validate().is_err());
        source.version = 2;
        source.provenance = MemorySourceProvenance::ManualNote;
        assert!(source.validate().is_err());
    }
}
