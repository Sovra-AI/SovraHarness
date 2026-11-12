//! Constant-memory replay protection for one authenticated phone connection.

use crate::{development::UiRequest, group::DeviceId};

use super::{ErrorCode, PROTOCOL_VERSION, RemoteOperation, RemoteRequest};

/// Renew the session after this many requests; the runtime supplies a new random
/// nonce on Hello. Old requests cannot become valid again when the counter resets.
pub const MAX_SESSION_REQUESTS: u32 = 4096;

#[derive(Debug)]
pub struct SessionGuard {
    generation: DeviceId,
    session: DeviceId,
    next_sequence: u32,
}

impl SessionGuard {
    pub fn new(generation: DeviceId, session: DeviceId) -> Self {
        Self {
            generation,
            session,
            next_sequence: 1,
        }
    }

    /// Runtime checks the authenticated peer against the live allowlist separately,
    /// before calling this. A consumed sequence is never reused, even on UI error.
    pub fn admit<'a>(&mut self, envelope: &'a RemoteRequest) -> Result<&'a UiRequest, ErrorCode> {
        if envelope.version != PROTOCOL_VERSION {
            return Err(ErrorCode::UnsupportedVersion);
        }
        let RemoteOperation::Control {
            generation,
            session,
            sequence,
            revision,
            request,
        } = &envelope.operation
        else {
            return Err(ErrorCode::InvalidRequest);
        };
        if *generation != self.generation
            || *session != self.session
            || *sequence != self.next_sequence
            || *sequence > MAX_SESSION_REQUESTS
        {
            return Err(ErrorCode::StaleSession);
        }
        self.next_sequence += 1;
        request.validate().map_err(|_| ErrorCode::InvalidRequest)?;
        let mutation = match request {
            UiRequest::Inspect {} | UiRequest::Screenshot {} => false,
            UiRequest::Click { .. }
            | UiRequest::Type { .. }
            | UiRequest::Key { .. }
            | UiRequest::Scroll { .. } => true,
        };
        if revision.as_ref().is_some_and(|value| {
            value.len() > 128 || value.is_empty() || value.chars().any(char::is_control)
        }) {
            return Err(ErrorCode::InvalidRequest);
        }
        if mutation && revision.is_none() {
            return Err(ErrorCode::StaleObservation);
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(sequence: u32) -> RemoteRequest {
        RemoteRequest {
            version: 1,
            operation: RemoteOperation::Control {
                generation: DeviceId([1; 32]),
                session: DeviceId([2; 32]),
                sequence,
                revision: None,
                request: UiRequest::Inspect {},
            },
        }
    }

    #[test]
    fn replay_gaps_and_old_connections_are_rejected_without_unbounded_ledger() {
        let mut guard = SessionGuard::new(DeviceId([1; 32]), DeviceId([2; 32]));
        assert_eq!(guard.admit(&control(0)), Err(ErrorCode::StaleSession));
        assert_eq!(guard.admit(&control(2)), Err(ErrorCode::StaleSession));
        assert!(guard.admit(&control(1)).is_ok());
        assert_eq!(guard.admit(&control(1)), Err(ErrorCode::StaleSession));
        for sequence in 2..=MAX_SESSION_REQUESTS {
            assert!(guard.admit(&control(sequence)).is_ok());
        }
        assert_eq!(
            guard.admit(&control(MAX_SESSION_REQUESTS + 1)),
            Err(ErrorCode::StaleSession)
        );
        let mut reconnected = SessionGuard::new(DeviceId([1; 32]), DeviceId([3; 32]));
        assert_eq!(reconnected.admit(&control(1)), Err(ErrorCode::StaleSession));
        let mut restarted = SessionGuard::new(DeviceId([3; 32]), DeviceId([2; 32]));
        assert_eq!(restarted.admit(&control(1)), Err(ErrorCode::StaleSession));
    }

    #[test]
    fn invalid_input_consumes_sequence_and_mutations_require_observation() {
        let mut guard = SessionGuard::new(DeviceId([1; 32]), DeviceId([2; 32]));
        let mut request = control(1);
        if let RemoteOperation::Control { request, .. } = &mut request.operation {
            *request = UiRequest::Type {
                text: "hello".into(),
            };
        }
        assert_eq!(guard.admit(&request), Err(ErrorCode::StaleObservation));
        assert_eq!(guard.admit(&request), Err(ErrorCode::StaleSession));
        if let RemoteOperation::Control {
            sequence, revision, ..
        } = &mut request.operation
        {
            *sequence = 2;
            *revision = Some("observed-revision".into());
        }
        assert!(guard.admit(&request).is_ok());
        if let RemoteOperation::Control {
            sequence, request, ..
        } = &mut request.operation
        {
            *sequence = 3;
            *request = UiRequest::Type { text: "\0".into() };
        }
        assert_eq!(guard.admit(&request), Err(ErrorCode::InvalidRequest));
        request.version = 2;
        assert_eq!(guard.admit(&request), Err(ErrorCode::UnsupportedVersion));
        request.version = 1;
        request.operation = RemoteOperation::Hello {};
        assert_eq!(guard.admit(&request), Err(ErrorCode::InvalidRequest));
    }
}
