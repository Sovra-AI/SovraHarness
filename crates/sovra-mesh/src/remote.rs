//! Dedicated encrypted phone endpoint. The runtime owns deadlines and task limits.

use std::sync::Arc;

use iroh::{
    Endpoint, SecretKey,
    endpoint::{self, presets},
};
use sovra_core::{
    group::{DeviceId, PeerAddress, TransportError},
    remote_control::{
        ErrorCode, IncomingRequest, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, REMOTE_ALPN,
        RemoteConnection, RemoteEndpoint, RemoteHandshake, RemoteReply, RemoteRequest,
        RemoteResponse,
    },
};

use crate::{Mesh, MeshError, Reach, peer_address};

/// Independent endpoint and key: closing phone access never closes Groups.
#[derive(Debug)]
pub struct RemoteMesh {
    endpoint: Endpoint,
}

impl RemoteMesh {
    pub async fn bind(secret: [u8; 32], reach: Reach) -> Result<Self, MeshError> {
        let builder = match reach {
            Reach::LocalOnly => Endpoint::builder(presets::Minimal),
            Reach::Internet => Endpoint::builder(presets::N0),
        };
        let endpoint = builder
            .secret_key(SecretKey::from_bytes(&secret))
            .alpns(vec![REMOTE_ALPN.to_vec()])
            .transport_config(
                endpoint::QuicTransportConfig::builder()
                    .max_concurrent_uni_streams(0u32.into())
                    .max_concurrent_bidi_streams(1u32.into())
                    .stream_receive_window(32768u32.into())
                    .receive_window(32768u32.into())
                    .build(),
            )
            .bind()
            .await
            .map_err(|error| MeshError::Bind(error.to_string()))?;
        Ok(Self { endpoint })
    }
}

#[derive(Debug)]
struct Handshake(endpoint::Incoming);

#[derive(Debug)]
struct Connection(endpoint::Connection);

// Cancelled runtime tasks must not leave a peer with an unowned open connection.
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"remote control ended");
    }
}

#[derive(Debug)]
struct Reply(endpoint::SendStream);

fn unreachable(error: impl std::fmt::Display) -> TransportError {
    TransportError::Unreachable(error.to_string())
}

fn encode<T: serde::Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, TransportError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| TransportError::Protocol("Could not encode remote message".into()))?;
    if bytes.len() > limit {
        return Err(TransportError::Protocol(
            "Remote message exceeds its size limit".into(),
        ));
    }
    Ok(bytes)
}

fn connection(
    connection: endpoint::Connection,
) -> Result<Arc<dyn RemoteConnection>, TransportError> {
    if connection.alpn() != REMOTE_ALPN {
        connection.close(0u32.into(), b"unsupported protocol");
        return Err(TransportError::Protocol(
            "Unsupported remote protocol".into(),
        ));
    }
    Ok(Arc::new(Connection(connection)))
}

#[async_trait::async_trait]
impl RemoteEndpoint for RemoteMesh {
    fn address(&self) -> PeerAddress {
        peer_address(&self.endpoint)
    }

    async fn accept(&self) -> Result<Box<dyn RemoteHandshake>, TransportError> {
        self.endpoint
            .accept()
            .await
            .map(|incoming| Box::new(Handshake(incoming)) as Box<dyn RemoteHandshake>)
            .ok_or(TransportError::Closed)
    }

    async fn connect(
        &self,
        address: &PeerAddress,
    ) -> Result<Arc<dyn RemoteConnection>, TransportError> {
        let established = self
            .endpoint
            .connect(Mesh::target(address)?, REMOTE_ALPN)
            .await
            .map_err(unreachable)?;
        connection(established)
    }

    async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[async_trait::async_trait]
impl RemoteHandshake for Handshake {
    async fn complete(self: Box<Self>) -> Result<Arc<dyn RemoteConnection>, TransportError> {
        connection(self.0.await.map_err(unreachable)?)
    }
}

#[async_trait::async_trait]
impl RemoteConnection for Connection {
    fn peer(&self) -> DeviceId {
        DeviceId(*self.0.remote_id().as_bytes())
    }

    async fn receive(&self) -> Result<IncomingRequest, TransportError> {
        let (send, mut recv) = self.0.accept_bi().await.map_err(unreachable)?;
        let bytes = recv.read_to_end(MAX_REQUEST_BYTES).await.map_err(|_| {
            TransportError::Protocol("Remote request is incomplete or too large".into())
        })?;
        if let Ok(request) = serde_json::from_slice(&bytes) {
            Ok(IncomingRequest {
                request,
                reply: Box::new(Reply(send)),
            })
        } else {
            Box::new(Reply(send))
                .send(RemoteResponse::Error {
                    code: ErrorCode::InvalidRequest,
                    message: "Malformed remote request.".into(),
                })
                .await?;
            Err(TransportError::Protocol("Malformed remote request".into()))
        }
    }

    async fn request(&self, request: &RemoteRequest) -> Result<RemoteResponse, TransportError> {
        let bytes = encode(request, MAX_REQUEST_BYTES)?;
        let (mut send, mut recv) = self.0.open_bi().await.map_err(unreachable)?;
        send.write_all(&bytes).await.map_err(unreachable)?;
        send.finish().map_err(unreachable)?;
        let bytes = recv.read_to_end(MAX_RESPONSE_BYTES).await.map_err(|_| {
            TransportError::Protocol("Remote reply is incomplete or too large".into())
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|_| TransportError::Protocol("Malformed remote reply".into()))
    }

    fn close(&self) {
        self.0.close(0u32.into(), b"remote control ended");
    }
}

#[async_trait::async_trait]
impl RemoteReply for Reply {
    async fn send(mut self: Box<Self>, response: RemoteResponse) -> Result<(), TransportError> {
        let bytes = encode(&response, MAX_RESPONSE_BYTES)?;
        self.0.write_all(&bytes).await.map_err(unreachable)?;
        self.0.finish().map_err(unreachable)?;
        // A malformed request closes its connection next. Wait for acknowledgement
        // so that close cannot discard the typed error still in the send buffer.
        // The runtime wraps this entire operation in its write deadline.
        if self.0.stopped().await.map_err(unreachable)?.is_some() {
            return Err(TransportError::Closed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
