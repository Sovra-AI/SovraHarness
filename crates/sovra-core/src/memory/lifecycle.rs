//! Bounded browsing and explicit user corrections, separate from automatic remembering.

use super::{Episode, Fact, FactId, MemoryError, Timestamp};

pub const MAX_MEMORY_PAGE: usize = 50;

/// A keyset boundary in descending creation-time/ID order, never an offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCursor {
    pub created_at: Timestamp,
    pub id: FactId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryListQuery {
    /// A literal, case-insensitive text fragment; empty means all memories.
    pub search: String,
    /// Exact saved subject label, selected from a displayed memory.
    pub subject: Option<String>,
    pub source: Option<super::EpisodeId>,
    pub include_history: bool,
    pub before: Option<MemoryCursor>,
    pub limit: usize,
}

impl Default for MemoryListQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            subject: None,
            source: None,
            include_history: false,
            before: None,
            limit: 20,
        }
    }
}

impl MemoryListQuery {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.limit == 0
            || self.limit > MAX_MEMORY_PAGE
            || self.search.len() > 512
            || self
                .source
                .as_ref()
                .is_some_and(|id| id.0.is_empty() || id.0.len() > 512)
            || self
                .subject
                .as_ref()
                .is_some_and(|subject| subject.trim().is_empty() || subject.len() > 256)
            || self
                .before
                .as_ref()
                .is_some_and(|cursor| cursor.id.0.trim().is_empty() || cursor.id.0.len() > 512)
        {
            return Err(MemoryError::InvalidInput(
                "invalid memory page request".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPage {
    pub facts: Vec<Fact>,
    pub next: Option<MemoryCursor>,
    /// Capture totals are not filtered by the visible search. `None` means the
    /// backend cannot report them, rather than claiming nothing was saved.
    pub overview: Option<super::MemoryOverview>,
    /// Chats already known for facts on this page. Empty when a chat was not recorded.
    pub chats: Vec<super::MemoryFactChat>,
}

/// An explicit edit creates a new user-authored note and a replacement fact atomically.
/// The target must still be current; editing an old version never revives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCorrection {
    pub target: FactId,
    pub text: String,
}

/// A detail the person explicitly asks to keep, without model interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryNote {
    pub text: String,
}

impl MemoryNote {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.text.trim().is_empty() || self.text.len() > 2048 {
            return Err(MemoryError::InvalidInput(
                "Write a short detail to remember. Please keep it to a few sentences.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAction {
    ConfigureSemantic(Option<crate::models::ModelDestination>),
    List(MemoryListQuery),
    ListSubjects(super::MemorySubjectQuery),
    ListSources(super::MemorySourceQuery),
    Inspect(FactId),
    InspectSource(super::EpisodeId),
    StopSource(super::EpisodeId),
    Correct(MemoryCorrection),
    Remember(MemoryNote),
    RetryFailed,
    StopUsing(FactId),
    /// Remove every correction version and its supporting sources from recall.
    Remove(FactId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryCommand {
    pub request_id: u64,
    pub action: MemoryAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDetail {
    pub fact: Fact,
    pub source: Option<Episode>,
    pub neighborhood: super::MemoryNeighborhood,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryReviewResult {
    Semantics(super::MemorySemanticState),
    Page(MemoryPage),
    Subjects(super::MemorySubjectPage),
    Detail(Box<MemoryDetail>),
    Source(super::MemorySourceRecord),
    Sources(super::MemorySourcePage),
    SourceStopped(super::EpisodeId),
    Changed(FactId),
    Retried(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryReviewEvent {
    pub request_id: u64,
    pub result: Result<MemoryReviewResult, String>,
}

/// One visible request at a time; old results cannot replace a newer search or edit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryReviewState {
    pub semantic: super::MemorySemanticStatus,
    pub page: MemoryPage,
    pub subjects: super::MemorySubjectPage,
    pub detail: Option<MemoryDetail>,
    pub source: Option<super::MemorySourceRecord>,
    pub sources: super::MemorySourcePage,
    pub error: Option<String>,
    pub loading: bool,
    request_id: u64,
}

impl MemoryReviewState {
    pub fn begin(&mut self, action: MemoryAction) -> MemoryCommand {
        self.request_id = self.request_id.wrapping_add(1);
        self.loading = true;
        self.error = None;
        self.source = None;
        MemoryCommand {
            request_id: self.request_id,
            action,
        }
    }

    /// Returns whether the event belongs to the active request.
    pub fn apply(&mut self, event: &MemoryReviewEvent) -> bool {
        if !self.loading || event.request_id != self.request_id {
            return false;
        }
        self.loading = false;
        match &event.result {
            Ok(MemoryReviewResult::Semantics(state)) => {
                // Worker progress may arrive before the command acknowledgement.
                if state.generation > self.semantic.state.generation {
                    self.semantic.apply(&super::MemorySemanticStatus {
                        state: state.clone(),
                        phase: if state.setting.is_some() {
                            super::MemorySemanticPhase::Updating
                        } else {
                            super::MemorySemanticPhase::Disabled
                        },
                    });
                }
            }
            Ok(MemoryReviewResult::Subjects(page)) => {
                self.subjects = page.clone();
                self.detail = None;
            }
            Ok(MemoryReviewResult::Page(page)) => {
                self.page = page.clone();
                self.detail = None;
            }
            Ok(MemoryReviewResult::Sources(page)) => {
                self.sources = page.clone();
                self.detail = None;
            }
            Ok(MemoryReviewResult::Source(source)) => {
                self.source = Some(source.clone());
                self.detail = None;
            }
            Ok(MemoryReviewResult::Detail(detail)) => self.detail = Some((**detail).clone()),
            Ok(
                MemoryReviewResult::Changed(_)
                | MemoryReviewResult::Retried(_)
                | MemoryReviewResult::SourceStopped(_),
            ) => {
                self.page = MemoryPage::default();
                self.detail = None;
                self.source = None;
            }
            Err(error) => self.error = Some(error.clone()),
        }
        true
    }
}

impl MemoryCorrection {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.target.0.trim().is_empty()
            || self.target.0.len() > 512
            || self.text.trim().is_empty()
            || self.text.len() > 2048
        {
            return Err(MemoryError::InvalidInput(
                "Enter a short correction for this saved detail.".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_notes_require_bounded_nonempty_text() {
        for text in [String::new(), "  \n ".into(), "é".repeat(1025)] {
            assert!(MemoryNote { text }.validate().is_err());
        }
        for text in [" I prefer tea without sugar ".into(), "é".repeat(1024)] {
            assert!(MemoryNote { text }.validate().is_ok());
        }
    }

    #[test]
    fn configuration_acknowledgement_does_not_erase_newer_worker_progress() {
        use crate::models::{ModelConnection, ModelDestination, ModelInfo, ProviderKind};
        use crate::{
            MemorySemanticPhase, MemorySemanticSetting, MemorySemanticState, MemorySemanticStatus,
        };
        let target = ModelDestination::for_embeddings(
            &ModelConnection::new("search", ProviderKind::Ollama),
            &ModelInfo {
                id: "embed".into(),
                output_modalities: vec!["embeddings".into()],
                ..Default::default()
            },
        )
        .unwrap();
        let selection = MemorySemanticState {
            generation: 1,
            setting: Some(MemorySemanticSetting {
                generation: 1,
                target: target.clone(),
                dimension: None,
            }),
        };
        let mut state = MemoryReviewState::default();
        let command = state.begin(MemoryAction::ConfigureSemantic(Some(target)));
        let mut ready = MemorySemanticStatus {
            state: selection.clone(),
            phase: MemorySemanticPhase::Ready,
        };
        ready.state.setting.as_mut().unwrap().dimension = Some(2);
        state.semantic.apply(&ready);
        assert!(state.apply(&MemoryReviewEvent {
            request_id: command.request_id,
            result: Ok(MemoryReviewResult::Semantics(selection))
        }));
        assert_eq!(state.semantic, ready);
        assert!(!state.loading);
    }

    #[test]
    fn late_review_results_cannot_replace_a_new_search() {
        let mut state = MemoryReviewState::default();
        let old = state.begin(MemoryAction::List(MemoryListQuery::default()));
        let new = state.begin(MemoryAction::List(MemoryListQuery {
            search: "tea".into(),
            ..Default::default()
        }));
        assert!(!state.apply(&MemoryReviewEvent {
            request_id: old.request_id,
            result: Err("stale".into())
        }));
        assert!(state.loading);
        assert!(state.error.is_none());
        assert!(state.apply(&MemoryReviewEvent {
            request_id: new.request_id,
            result: Err("try again".into())
        }));
        assert!(!state.loading);
        assert_eq!(state.error.as_deref(), Some("try again"));
        assert!(!state.apply(&MemoryReviewEvent {
            request_id: new.request_id,
            result: Ok(MemoryReviewResult::Page(MemoryPage::default()))
        }));
    }

    #[test]
    fn browsing_and_corrections_have_explicit_bounds() {
        assert!(MemoryListQuery::default().validate().is_ok());
        for limit in [0, MAX_MEMORY_PAGE + 1, usize::MAX] {
            assert!(
                MemoryListQuery {
                    limit,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            MemoryListQuery {
                search: "茶".repeat(171),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            MemoryListQuery {
                before: Some(MemoryCursor {
                    created_at: Timestamp(1),
                    id: FactId(String::new())
                }),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let mut correction = MemoryCorrection {
            target: FactId("fact:one".into()),
            text: "I prefer jasmine tea".into(),
        };
        assert!(correction.validate().is_ok());
        correction.text = " ".into();
        assert!(correction.validate().is_err());
        correction.text = "茶".repeat(683);
        assert!(correction.validate().is_err());
    }
}
