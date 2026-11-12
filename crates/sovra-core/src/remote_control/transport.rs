//! Runtime-independent transport ports. The runtime owns tasks and deadlines.

use std::sync::Arc;

use crate::group::{DeviceId, PeerAddress, TransportError};

use super::{RemoteRequest, RemoteResponse};

/// Accepting an incoming attempt does not wait for its handshake; the runtime can
/// bound concurrent handshakes and apply an independent deadline to each one.
#[async_trait::async_trait]
pub trait RemoteEndpoint: Send + Sync + std::fmt::Debug {
    fn address(&self) -> PeerAddress;
    async fn accept(&self) -> Result<Box<dyn RemoteHandshake>, TransportError>;
    async fn connect(
        &self,
        address: &PeerAddress,
    ) -> Result<Arc<dyn RemoteConnection>, TransportError>;
    async fn close(&self);
}

#[async_trait::async_trait]
pub trait RemoteHandshake: Send + std::fmt::Debug {
    async fn complete(self: Box<Self>) -> Result<Arc<dyn RemoteConnection>, TransportError>;
}

#[async_trait::async_trait]
pub trait RemoteConnection: Send + Sync + std::fmt::Debug {
    /// Identity from a completed handshake, never a client-supplied message field.
    fn peer(&self) -> DeviceId;
    async fn receive(&self) -> Result<IncomingRequest, TransportError>;
    /// Client-side operation used by reference clients and interoperability tests.
    async fn request(&self, request: &RemoteRequest) -> Result<RemoteResponse, TransportError>;
    fn close(&self);
}

#[derive(Debug)]
pub struct IncomingRequest {
    pub request: RemoteRequest,
    pub reply: Box<dyn RemoteReply>,
}

#[async_trait::async_trait]
pub trait RemoteReply: Send + std::fmt::Debug {
    async fn send(self: Box<Self>, response: RemoteResponse) -> Result<(), TransportError>;
}
