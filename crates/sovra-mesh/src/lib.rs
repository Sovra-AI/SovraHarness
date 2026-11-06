//! Peer-to-peer adapter for groups: the only crate that names `iroh`.
//!
//! Owns this computer's device key (an Ed25519 secret that never leaves the
//! adapter), dials other computers by their public key over QUIC with hole punching
//! and an encrypted relay fallback, and carries [`PeerRequest`]s one per stream.
//! It decides nothing: every rule (who may join, sync or push, how history folds)
//! lives in `sovra_core::group`, and every task runs on the spawner the runtime
//! hands in, so the runtime owns cancellation. This crate never names `tokio`.

mod store;

use std::str::FromStr as _;
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, SecretKey, TransportAddr};
use sovra_core::group::{
    DeviceId, GroupTransport, MAX_PEER_MESSAGE_BYTES, PEER_ALPN, PeerAddress, PeerHandler,
    PeerRequest, PeerResponse, Signature, SignatureVerifier, Spawn, TransportError,
};

pub use store::FileGroupStore;

/// How far this computer reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Direct connections only: the same network, or tests. No relays, no lookups.
    LocalOnly,
    /// Relays and key-based address lookup, so computers find each other through
    /// home routers. Development uses n0's public relays (not for production; see
    /// roadmap decision D4).
    Internet,
}

/// Why the mesh could not start.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error("couldn't open the network connection: {0}")]
    Bind(String),
}

/// Ed25519 checks for group operations, using the same keys the transport dials.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ed25519Verifier;

impl SignatureVerifier for Ed25519Verifier {
    fn verify(&self, signer: &DeviceId, message: &[u8], signature: &Signature) -> bool {
        PublicKey::from_bytes(signer.as_bytes()).is_ok_and(|key| {
            key.verify(message, &iroh::Signature::from_bytes(&signature.0))
                .is_ok()
        })
    }
}

/// This computer's place in the peer-to-peer network.
#[derive(Debug, Clone)]
pub struct Mesh {
    endpoint: Endpoint,
    secret: SecretKey,
}

impl Mesh {
    /// Binds the endpoint. Must run inside the runtime's async reactor.
    pub async fn bind(device_secret: [u8; 32], reach: Reach) -> Result<Self, MeshError> {
        let secret = SecretKey::from_bytes(&device_secret);
        let builder = match reach {
            Reach::LocalOnly => Endpoint::builder(presets::Minimal),
            Reach::Internet => Endpoint::builder(presets::N0),
        };
        let endpoint = builder
            .secret_key(secret.clone())
            .alpns(vec![PEER_ALPN.to_vec()])
            .bind()
            .await
            .map_err(|error| MeshError::Bind(error.to_string()))?;
        tracing::info!(device = %endpoint.id().fmt_short(), ?reach, "group network ready");
        Ok(Self { endpoint, secret })
    }

    /// The public key for a device secret, without binding anything.
    pub fn device_for(device_secret: &[u8; 32]) -> DeviceId {
        DeviceId(*SecretKey::from_bytes(device_secret).public().as_bytes())
    }

    fn target(to: &PeerAddress) -> Result<EndpointAddr, TransportError> {
        let key = PublicKey::from_bytes(to.device.as_bytes())
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let mut addrs = Vec::new();
        if let Some(relay) = &to.relay
            && let Ok(url) = RelayUrl::from_str(relay)
        {
            addrs.push(TransportAddr::Relay(url));
        }
        addrs.extend(
            to.addrs
                .iter()
                .filter_map(|text| text.parse().ok())
                .map(TransportAddr::Ip),
        );
        Ok(EndpointAddr::from_parts(key, addrs))
    }
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, TransportError> {
    serde_json::from_slice(bytes).map_err(|error| TransportError::Protocol(error.to_string()))
}

fn encode<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("peer messages are plain serialisable data")
}

/// Serves every stream on one accepted connection until the peer closes it.
async fn serve_connection(incoming: iroh::endpoint::Incoming, handler: Arc<dyn PeerHandler>) {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(%error, "incoming group connection failed");
            return;
        }
    };
    let from = DeviceId(*connection.remote_id().as_bytes());
    loop {
        let Ok((mut send, mut recv)) = connection.accept_bi().await else {
            return;
        };
        let response = match recv.read_to_end(MAX_PEER_MESSAGE_BYTES).await {
            Ok(bytes) => match decode::<PeerRequest>(&bytes) {
                Ok(request) => handler.handle(from, request).await,
                Err(_) => PeerResponse::Refused {
                    reason: sovra_core::group::Refusal::Malformed,
                },
            },
            Err(error) => {
                tracing::debug!(%error, "group request could not be read");
                return;
            }
        };
        if send.write_all(&encode(&response)).await.is_err() || send.finish().is_err() {
            return;
        }
    }
}

#[async_trait::async_trait]
impl GroupTransport for Mesh {
    fn this_device(&self) -> DeviceId {
        DeviceId(*self.endpoint.id().as_bytes())
    }

    fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.secret.sign(message).to_bytes())
    }

    fn verifier(&self) -> Arc<dyn SignatureVerifier + Send + Sync> {
        Arc::new(Ed25519Verifier)
    }

    fn address(&self) -> PeerAddress {
        let addr = self.endpoint.addr();
        PeerAddress {
            device: self.this_device(),
            relay: addr.relay_urls().next().map(ToString::to_string),
            addrs: addr.ip_addrs().map(ToString::to_string).collect(),
        }
    }

    async fn serve(&self, handler: Arc<dyn PeerHandler>, spawn: Spawn) {
        while let Some(incoming) = self.endpoint.accept().await {
            spawn(Box::pin(serve_connection(incoming, handler.clone())));
        }
    }

    async fn request(
        &self,
        to: &PeerAddress,
        request: PeerRequest,
    ) -> Result<PeerResponse, TransportError> {
        if self.endpoint.is_closed() {
            return Err(TransportError::Closed);
        }
        let connection = self
            .endpoint
            .connect(Self::target(to)?, PEER_ALPN)
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        send.write_all(&encode(&request))
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        send.finish()
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        let bytes = recv
            .read_to_end(MAX_PEER_MESSAGE_BYTES)
            .await
            .map_err(|error| TransportError::Protocol(error.to_string()))?;
        connection.close(0u32.into(), b"done");
        decode(&bytes)
    }

    async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests;
