//! Opt-in development control: typed UI requests and pure validation.

use serde::{Deserialize, Serialize};

/// Bounds keep local clients from monopolising the UI thread or allocating freely.
pub const MAX_TEXT_BYTES: usize = 4096;
pub const MAX_TARGET_BYTES: usize = 2048;
pub const MAX_ELEMENTS: usize = 1000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevelopmentState {
    pub enabled: bool,
    /// Changes on every activation, including application restarts.
    pub generation: Option<String>,
    pub error: Option<String>,
    /// Executable and arguments only; never the private endpoint or its credential.
    pub connection_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevelopmentCommand {
    SetEnabled(bool),
    UiReply {
        generation: String,
        request_id: u64,
        result: Result<serde_json::Value, String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevelopmentEvent {
    State(DevelopmentState),
    UiRequest {
        generation: String,
        request_id: u64,
        request: UiRequest,
        deadline: std::time::Instant,
    },
}

/// Input goes through native hit testing and key bindings; there is no evaluation
/// endpoint, arbitrary command dispatch, filesystem access, or remote mode switch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum UiRequest {
    Inspect {},
    Screenshot {},
    Click { target: String },
    Type { text: String },
    Key { key: String },
    Scroll { target: String, lines: i16 },
}

impl UiRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Inspect {} | Self::Screenshot {} => Ok(()),
            Self::Click { target } | Self::Scroll { target, .. }
                if target.is_empty() || target.len() > MAX_TARGET_BYTES =>
            {
                Err("target must be a nonempty ID from inspect, at most 2048 bytes")
            }
            Self::Scroll { lines, .. } if lines.unsigned_abs() > 100 => {
                Err("scroll must be between -100 and 100 lines")
            }
            Self::Type { text } if text.len() > MAX_TEXT_BYTES || text.contains('\0') => {
                Err("text must be at most 4096 bytes and contain no NUL")
            }
            Self::Key { key } if key.is_empty() || key.len() > 80 => {
                Err("key must be a nonempty GPUI keystroke of at most 80 bytes")
            }
            Self::Click { .. } | Self::Type { .. } | Self::Key { .. } | Self::Scroll { .. } => {
                Ok(())
            }
        }
    }
}

impl DevelopmentState {
    pub fn apply(&mut self, event: &DevelopmentEvent) {
        match event {
            DevelopmentEvent::State(state) => *self = state.clone(),
            DevelopmentEvent::UiRequest { .. } => {}
        }
    }

    pub fn accepts(&self, generation: &str) -> bool {
        self.enabled && self.generation.as_deref() == Some(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_requires_enabled_and_current_generation() {
        let mut state = DevelopmentState::default();
        assert!(!state.accepts("first"));
        state.apply(&DevelopmentEvent::State(DevelopmentState {
            enabled: true,
            generation: Some("first".into()),
            ..DevelopmentState::default()
        }));
        assert!(state.accepts("first"));
        assert!(!state.accepts("stale"));
        state.apply(&DevelopmentEvent::State(DevelopmentState::default()));
        assert!(!state.accepts("first"));
    }

    #[test]
    fn requests_are_bounded_and_reject_extra_fields() {
        assert!(
            UiRequest::Type {
                text: "x".repeat(MAX_TEXT_BYTES)
            }
            .validate()
            .is_ok()
        );
        assert!(
            UiRequest::Type {
                text: "x".repeat(MAX_TEXT_BYTES + 1)
            }
            .validate()
            .is_err()
        );
        assert!(
            UiRequest::Scroll {
                target: "pane".into(),
                lines: i16::MIN
            }
            .validate()
            .is_err()
        );
        assert!(
            UiRequest::Click {
                target: String::new()
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_str::<UiRequest>(r#"{"action":"inspect","enable":true}"#).is_err()
        );
    }
}
