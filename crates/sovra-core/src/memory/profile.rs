//! Persisted capture health and the person's current saved knowledge.

use super::{EpisodeId, MemoryJobState, Timestamp};

/// Global totals, independent of the current search or page of saved details.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryOverview {
    pub current: usize,
    pub history: usize,
    pub stopped: usize,
    pub waiting: usize,
    pub working: usize,
    pub failed: usize,
    pub checked: usize,
    pub cancelled: usize,
    pub last_saved_at: Option<Timestamp>,
    pub recent_activity: Vec<MemoryActivity>,
    /// Most-mentioned subjects across all current details, independent of this page.
    pub subjects: Vec<MemorySubject>,
    pub more_subjects: bool,
}

/// A stored subject label, not a model-inferred relationship or topic category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySubject {
    pub name: String,
    pub details: usize,
}

/// Alphabetical keyset browsing across current saved subject labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySubjectQuery {
    pub search: String,
    pub after: Option<String>,
    pub limit: usize,
}

impl Default for MemorySubjectQuery {
    fn default() -> Self {
        Self {
            search: String::new(),
            after: None,
            limit: 20,
        }
    }
}

impl MemorySubjectQuery {
    pub fn validate(&self) -> Result<(), super::MemoryError> {
        if self.search.len() > 256
            || self.limit == 0
            || self.limit > super::MAX_MEMORY_PAGE
            || self
                .after
                .as_ref()
                .is_some_and(|name| name.trim().is_empty() || name.len() > 256)
        {
            return Err(super::MemoryError::InvalidInput(
                "invalid subject directory request".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemorySubjectPage {
    pub subjects: Vec<MemorySubject>,
    pub next: Option<String>,
}

/// A bounded receipt survives reopening; it never implies that an empty check
/// saved a fact, or that a queued message has already been understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryActivity {
    pub episode_id: EpisodeId,
    pub session_id: String,
    pub created_at: Timestamp,
    pub state: MemoryJobState,
    pub saved: usize,
    pub error: Option<String>,
}

impl MemoryActivity {
    pub fn label(&self) -> String {
        match self.state {
            MemoryJobState::Pending => "Message saved — details waiting to be checked".into(),
            MemoryJobState::Processing => "Message saved — checking its details".into(),
            MemoryJobState::Complete => match self.saved {
                0 => "Checked — no new personal details found".into(),
                1 => "1 saved detail".into(),
                count => format!("{count} saved details"),
            },
            MemoryJobState::Failed => self
                .error
                .clone()
                .unwrap_or_else(|| "Could not finish — try again".into()),
            MemoryJobState::Cancelled => "Stopped when remembering was paused".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_requests_are_bounded_and_accept_unicode_cursors() {
        assert!(MemorySubjectQuery::default().validate().is_ok());
        assert!(
            MemorySubjectQuery {
                search: "茶".into(),
                after: Some("École".into()),
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
        for query in [
            MemorySubjectQuery {
                limit: 0,
                ..Default::default()
            },
            MemorySubjectQuery {
                limit: 51,
                ..Default::default()
            },
            MemorySubjectQuery {
                search: "茶".repeat(86),
                ..Default::default()
            },
            MemorySubjectQuery {
                after: Some(" ".into()),
                ..Default::default()
            },
            MemorySubjectQuery {
                after: Some("a".repeat(257)),
                ..Default::default()
            },
        ] {
            assert!(query.validate().is_err());
        }
    }

    #[test]
    fn activity_distinguishes_checks_from_saved_details_and_unfinished_work() {
        let mut activity = MemoryActivity {
            episode_id: EpisodeId("episode:fixture".into()),
            session_id: "fixture".into(),
            created_at: Timestamp(1),
            state: MemoryJobState::Complete,
            saved: 0,
            error: None,
        };
        assert!(activity.label().contains("no new personal details"));
        activity.saved = 1;
        assert_eq!(activity.label(), "1 saved detail");
        activity.saved = 3;
        assert_eq!(activity.label(), "3 saved details");
        for state in [
            MemoryJobState::Pending,
            MemoryJobState::Processing,
            MemoryJobState::Cancelled,
        ] {
            activity.state = state;
            if state == MemoryJobState::Cancelled {
                assert!(!activity.label().contains("saved"));
            } else {
                assert!(activity.label().contains("Message saved"));
            }
        }
        activity.state = MemoryJobState::Failed;
        assert!(activity.label().contains("try again"));
        activity.error = Some("Choose a working AI in Models.".into());
        assert_eq!(activity.label(), "Choose a working AI in Models.");
    }
}
