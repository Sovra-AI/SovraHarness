//! Desktop commands, observable status and the UI dispatch gate for phone access.

use super::{ErrorCode, PairedPhone, PairingTicket};
use crate::{development::UiRequest, group::DeviceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFailure {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteCommand {
    Start,
    Stop,
    PairAnother,
    Forget {
        device: DeviceId,
    },
    UiReply {
        generation: DeviceId,
        request_id: u64,
        result: Result<serde_json::Value, RemoteFailure>,
    },
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct RemoteState {
    pub generation: Option<DeviceId>,
    pub phones: Vec<PairedPhone>,
    pub invitation: Option<PairingTicket>,
    pub qr: Vec<Vec<bool>>,
    pub invitation_expires_at: Option<std::time::Instant>,
    pub connected_phone: Option<DeviceId>,
    pub session: Option<DeviceId>,
    pub error: Option<String>,
}

impl std::fmt::Debug for RemoteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A QR matrix is also a credential. Never derive Debug over it.
        f.debug_struct("RemoteState")
            .field("enabled", &self.generation.is_some())
            .field("phones", &self.phones.len())
            .field("pairing", &self.invitation.is_some())
            .field("connected", &self.connected_phone.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteEvent {
    State(RemoteState),
    Error(String),
    UiRequest {
        generation: DeviceId,
        phone: DeviceId,
        session: DeviceId,
        request_id: u64,
        revision: Option<String>,
        request: UiRequest,
        deadline: std::time::Instant,
    },
}

impl RemoteState {
    pub fn apply(&mut self, event: &RemoteEvent) {
        match event {
            RemoteEvent::State(state) => *self = state.clone(),
            RemoteEvent::Error(error) => self.error = Some(error.clone()),
            RemoteEvent::UiRequest { .. } => {}
        }
    }

    pub fn accepts(&self, generation: DeviceId, phone: DeviceId, session: DeviceId) -> bool {
        self.generation == Some(generation)
            // Pairing QR images are bearer credentials, including in screenshots.
            && self.invitation.is_none()
            && self.connected_phone == Some(phone)
            && self.session == Some(session)
            && self.phones.iter().any(|paired| paired.device == phone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_gate_requires_the_current_phone_session_and_activation() {
        let id = DeviceId([1; 32]);
        let other = DeviceId([2; 32]);
        let mut state = RemoteState::default();
        assert!(!state.accepts(id, id, id));
        state.apply(&RemoteEvent::State(RemoteState {
            generation: Some(id),
            connected_phone: Some(id),
            session: Some(id),
            phones: vec![PairedPhone {
                device: id,
                name: "Phone".into(),
            }],
            ..RemoteState::default()
        }));
        assert!(state.accepts(id, id, id));
        assert!(!state.accepts(other, id, id));
        assert!(!state.accepts(id, other, id));
        assert!(!state.accepts(id, id, other));
        state.invitation = Some(PairingTicket {
            version: 1,
            address: crate::group::PeerAddress {
                device: other,
                relay: None,
                addrs: Vec::new(),
            },
            secret: super::super::InvitationSecret([3; 32]),
        });
        assert!(
            !state.accepts(id, id, id),
            "An existing phone must not capture another phone's pairing QR"
        );
        state.invitation = None;
        assert!(state.accepts(id, id, id));
        state.phones.clear();
        assert!(!state.accepts(id, id, id));
        state.apply(&RemoteEvent::Error("Failure".into()));
        assert_eq!(state.error.as_deref(), Some("Failure"));
        assert!(!format!("{state:?}").contains("qr"));
    }
}
