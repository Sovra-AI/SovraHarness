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
            MemoryJobState::Pending => "Waiting to learn from this message".into(),
            MemoryJobState::Processing => "Reading this message".into(),
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
            assert!(!activity.label().contains("saved"));
        }
        activity.state = MemoryJobState::Failed;
        assert!(activity.label().contains("try again"));
        activity.error = Some("Choose a working AI in Models.".into());
        assert_eq!(activity.label(), "Choose a working AI in Models.");
    }
}
