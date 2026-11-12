//! Versioned JSON wire data shared with the future phone app.

use serde::{Deserialize, Serialize};

use crate::{
    development::UiRequest,
    group::{DeviceId, PeerAddress},
};

use super::{InvitationSecret, PROTOCOL_VERSION};

pub const MAX_REQUEST_BYTES: usize = 32 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
// Fits a QR byte payload at the encoder's default medium error correction.
const MAX_TICKET_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteRequest {
    pub version: u16,
    pub operation: RemoteOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteOperation {
    Pair {
        secret: InvitationSecret,
        name: String,
    },
    Hello {},
    Control {
        /// Random 32-byte values serialized as 64 hex digits, like endpoint IDs.
        generation: DeviceId,
        session: DeviceId,
        /// Monotonically increasing within the connection, starting at 1.
        sequence: u32,
        /// Returned by Inspect. Required for mutations, omitted for reads.
        revision: Option<String>,
        request: UiRequest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Disabled,
    UnsupportedVersion,
    NotPaired,
    InvalidInvitation,
    InvalidRequest,
    StaleSession,
    StaleObservation,
    Busy,
    StorageUnavailable,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteResponse {
    Paired {},
    Ready {
        version: u16,
        generation: DeviceId,
        session: DeviceId,
        max_request_bytes: usize,
        max_response_bytes: usize,
    },
    Result {
        value: serde_json::Value,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("This phone invitation is invalid or uses an unsupported version.")]
pub struct WireError;

/// QR payload. Debug redacts the secret through `InvitationSecret`; the encoded
/// link itself is a credential and must never enter logs or persisted history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingTicket {
    pub version: u16,
    pub address: PeerAddress,
    pub secret: InvitationSecret,
}

impl PairingTicket {
    pub fn encode(&self) -> Result<String, WireError> {
        self.validate()?;
        let payload = serde_json::to_string(self).map_err(|_| WireError)?;
        let mut link = url::Url::parse("sovra://remote-control").map_err(|_| WireError)?;
        link.query_pairs_mut().append_pair("ticket", &payload);
        let link: String = link.into();
        if link.len() > MAX_TICKET_BYTES {
            return Err(WireError);
        }
        Ok(link)
    }

    pub fn decode(link: &str) -> Result<Self, WireError> {
        if link.len() > MAX_TICKET_BYTES {
            return Err(WireError);
        }
        let url = url::Url::parse(link).map_err(|_| WireError)?;
        if url.scheme() != "sovra"
            || url.host_str() != Some("remote-control")
            || !url.path().is_empty()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(WireError);
        }
        let mut pairs = url.query_pairs();
        let (key, payload) = pairs.next().ok_or(WireError)?;
        if key != "ticket" || pairs.next().is_some() {
            return Err(WireError);
        }
        let ticket: Self = serde_json::from_str(&payload).map_err(|_| WireError)?;
        ticket.validate()?;
        Ok(ticket)
    }

    fn validate(&self) -> Result<(), WireError> {
        if self.version != PROTOCOL_VERSION || self.address.addrs.len() > 16 {
            return Err(WireError);
        }
        for address in &self.address.addrs {
            if address.parse::<std::net::SocketAddr>().is_err() {
                return Err(WireError);
            }
        }
        if let Some(relay) = &self.address.relay {
            if relay.len() > 2048 {
                return Err(WireError);
            }
            let relay = url::Url::parse(relay).map_err(|_| WireError)?;
            if relay.scheme() != "https"
                || relay.host_str().is_none()
                || !relay.username().is_empty()
                || relay.password().is_some()
                || relay.fragment().is_some()
                || relay.query().is_some()
            {
                return Err(WireError);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_mobile_fixtures_match_the_wire_schema() {
        for text in [
            include_str!("fixtures/hello-request.json"),
            include_str!("fixtures/pair-request.json"),
            include_str!("fixtures/inspect-request.json"),
        ] {
            let request: RemoteRequest = serde_json::from_str(text).unwrap();
            assert_eq!(request.version, PROTOCOL_VERSION);
            assert_eq!(
                serde_json::to_value(request).unwrap(),
                serde_json::from_str::<serde_json::Value>(text).unwrap()
            );
        }
        for text in [
            include_str!("fixtures/ready-response.json"),
            include_str!("fixtures/error-response.json"),
        ] {
            let response: RemoteResponse = serde_json::from_str(text).unwrap();
            assert_eq!(
                serde_json::to_value(response).unwrap(),
                serde_json::from_str::<serde_json::Value>(text).unwrap()
            );
        }
    }

    #[test]
    fn ticket_roundtrip_is_strict_and_debug_hides_secret() {
        let ticket = PairingTicket {
            version: 1,
            address: PeerAddress {
                device: DeviceId([1; 32]),
                relay: Some("https://relay.example/".into()),
                addrs: vec!["127.0.0.1:1234".into()],
            },
            secret: InvitationSecret([2; 32]),
        };
        let link = ticket.encode().unwrap();
        assert_eq!(PairingTicket::decode(&link).unwrap(), ticket);
        assert!(!format!("{ticket:?}").contains(&DeviceId([2; 32]).to_hex()));
        for bad in [
            format!("{link}&extra=x"),
            format!("{link}#fragment"),
            link.replace("sovra:", "https:"),
            "x".repeat(MAX_TICKET_BYTES + 1),
        ] {
            assert!(PairingTicket::decode(&bad).is_err());
        }
        let mut bad = ticket;
        bad.version = 2;
        assert!(bad.encode().is_err());
        bad.version = 1;
        bad.address.relay = Some("http://relay.example".into());
        assert!(bad.encode().is_err());
        bad.address.relay = None;
        bad.address.addrs = vec!["not an address".into()];
        assert!(bad.encode().is_err());
    }

    #[test]
    fn request_schema_rejects_unknown_commands_and_identity_spoofing() {
        let hello = r#"{"version":1,"operation":{"op":"hello"}}"#;
        let request: RemoteRequest = serde_json::from_str(hello).unwrap();
        assert_eq!(serde_json::to_string(&request).unwrap(), hello);
        for bad in [
            r#"{"version":1,"operation":{"op":"enable"}}"#,
            r#"{"version":1,"operation":{"op":"hello","device_id":"spoof"}}"#,
            r#"{"version":1,"operation":{"op":"hello"},"extra":true}"#,
        ] {
            assert!(serde_json::from_str::<RemoteRequest>(bad).is_err());
        }
        // New versions remain parseable so the runtime can return a typed error.
        assert_eq!(
            serde_json::from_str::<RemoteRequest>(&hello.replace(":1", ":2"))
                .unwrap()
                .version,
            2
        );
    }
}
