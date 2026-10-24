use crate::Event;

/// What the UI knows about the runtime, folded from [`Event`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RuntimeStatus {
    #[default]
    Starting,
    Ready {
        version: String,
    },
    Stopped,
}

impl RuntimeStatus {
    /// Applies one runtime event. Pure: the UI calls this and re-renders.
    pub fn apply(&mut self, event: &Event) {
        match event {
            Event::Ready { version } => {
                *self = Self::Ready {
                    version: version.clone(),
                };
            }
            Event::Models(_)
            | Event::ChatHistoryLoaded { .. }
            | Event::Pong
            | Event::RouteEvaluated { .. }
            | Event::AgentChunk { .. }
            | Event::AgentThinkingChunk { .. }
            | Event::AgentResponseStarted { .. }
            | Event::AgUiSurface { .. }
            | Event::AgentCompleted { .. }
            | Event::AgentError { .. } => {}
            Event::Stopped => *self = Self::Stopped,
        }
    }

    /// Short human-readable label for status displays.
    pub fn label(&self) -> String {
        match self {
            Self::Starting => "Starting…".to_owned(),
            Self::Ready { version } => format!("Ready · v{version}"),
            Self::Stopped => "Stopped".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_starting_state() {
        assert_eq!(RuntimeStatus::default(), RuntimeStatus::Starting);
        assert_eq!(RuntimeStatus::default().label(), "Starting…");
    }

    #[test]
    fn folds_lifecycle_events() {
        let mut status = RuntimeStatus::default();

        status.apply(&Event::Ready {
            version: "0.1.0".into(),
        });
        assert_eq!(status.label(), "Ready · v0.1.0");

        status.apply(&Event::Pong);
        assert_eq!(
            status.label(),
            "Ready · v0.1.0",
            "pong must not change status"
        );

        status.apply(&Event::AgentChunk {
            session_id: "s1".into(),
            text: "hello".into(),
        });
        status.apply(&Event::AgentCompleted {
            session_id: "s1".into(),
        });
        status.apply(&Event::AgentError {
            session_id: "s1".into(),
            message: "err".into(),
        });
        assert_eq!(
            status.label(),
            "Ready · v0.1.0",
            "agent turn events must not clobber runtime ready status"
        );

        status.apply(&Event::Stopped);
        assert_eq!(status, RuntimeStatus::Stopped);
    }
}
