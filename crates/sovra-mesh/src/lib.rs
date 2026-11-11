//! Peer-to-peer adapter for groups: the only crate that names `iroh`.
//!
//! Owns this computer's device key (an Ed25519 secret that never leaves the
//! adapter), dials other computers by their public key over QUIC with hole punching
//! and an encrypted relay fallback, and carries [`PeerRequest`]s one per stream.
//! It decides nothing: every rule (who may join, sync or push, how history folds)
//! lives in `sovra_core::group`, and every task runs on the spawner the runtime
//! hands in, so the runtime owns cancellation. This crate never names `tokio`.

mod cipher;
mod store;
mod swarm;

use std::str::FromStr as _;
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, SecretKey, TransportAddr};
use sovra_core::group::{
    DeviceId, GroupTransport, MAX_PEER_MESSAGE_BYTES, PEER_ALPN, PeerAddress, PeerHandler,
    PeerRequest, PeerResponse, Signature, SignatureVerifier, Spawn, TransportError,
};

pub use store::FileGroupStore;
pub use swarm::{OsRandom, SwarmKeys};

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
    cipher: Arc<cipher::MeshCipher>,
    /// Finds Sovra computers on the same network (same-room codes). `None` when the
    /// network allows no multicast; codes then say nobody is nearby.
    nearby: Option<iroh_mdns_address_lookup::MdnsAddressLookup>,
    /// Computers heard on this network and the tag each announces, kept up to date
    /// from the moment the group service starts. Discovery only reports new or
    /// changed computers, so a code shown before someone presses Join is still here.
    heard: Arc<std::sync::Mutex<std::collections::BTreeMap<DeviceId, Heard>>>,
    /// Broadcast trees (iroh-gossip) on this same endpoint, for audit checkpoints.
    gossip: iroh_gossip::net::Gossip,
    /// Addresses the group already knows, so gossip can dial members by key alone.
    known: iroh::address_lookup::memory::MemoryLookup,
    /// Joined topics and their senders.
    topics:
        Arc<std::sync::Mutex<std::collections::BTreeMap<[u8; 32], iroh_gossip::api::GossipSender>>>,
}

/// One computer heard on this network.
#[derive(Debug, Clone)]
struct Heard {
    address: PeerAddress,
    tag: Option<String>,
}

/// The mDNS service Sovra computers announce under (not iroh's shared default).
const NEARBY_SERVICE: &str = "sovra";
/// The tag a computer announces while it shows a same-room code, before its hint.
const PAIRING_TAG: &str = "sovra-pair-";

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
            .alpns(vec![
                PEER_ALPN.to_vec(),
                iroh_gossip::net::GOSSIP_ALPN.to_vec(),
            ])
            .bind()
            .await
            .map_err(|error| MeshError::Bind(error.to_string()))?;
        let gossip = iroh_gossip::net::Gossip::builder().spawn(endpoint.clone());
        let known = iroh::address_lookup::memory::MemoryLookup::new();
        tracing::info!(device = %endpoint.id().fmt_short(), ?reach, "group network ready");
        let cipher = Arc::new(cipher::MeshCipher::new(&device_secret));
        let nearby = iroh_mdns_address_lookup::MdnsAddressLookup::builder()
            .service_name(NEARBY_SERVICE)
            .build(endpoint.id())
            .map_err(|error| tracing::info!(%error, "same-room codes unavailable on this network"))
            .ok();
        if let Ok(lookup) = endpoint.address_lookup() {
            if let Some(nearby) = &nearby {
                lookup.add(nearby.clone());
            }
            lookup.add(known.clone());
        }
        Ok(Self {
            endpoint,
            secret,
            cipher,
            nearby,
            heard: Arc::default(),
            gossip,
            known,
            topics: Arc::default(),
        })
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

/// Records every computer discovery reports, with its announced tag, until shutdown.
async fn listen_nearby(
    nearby: iroh_mdns_address_lookup::MdnsAddressLookup,
    heard: Arc<std::sync::Mutex<std::collections::BTreeMap<DeviceId, Heard>>>,
    me: DeviceId,
) {
    use n0_future::StreamExt as _;
    let mut events = nearby.subscribe().await;
    while let Some(event) = events.next().await {
        let mut table = heard.lock().expect("nearby table lock is never poisoned");
        match event {
            iroh_mdns_address_lookup::DiscoveryEvent::Discovered { endpoint_info, .. } => {
                let device = DeviceId(*endpoint_info.endpoint_id.as_bytes());
                if device == me {
                    continue;
                }
                let tag = endpoint_info
                    .data
                    .user_data()
                    .map(|tag| tag.as_ref().to_owned());
                let addr = endpoint_info.to_endpoint_addr();
                table.insert(
                    device,
                    Heard {
                        address: PeerAddress {
                            device,
                            relay: addr.relay_urls().next().map(ToString::to_string),
                            addrs: addr.ip_addrs().map(ToString::to_string).collect(),
                        },
                        tag,
                    },
                );
            }
            iroh_mdns_address_lookup::DiscoveryEvent::Expired { endpoint_id } => {
                table.remove(&DeviceId(*endpoint_id.as_bytes()));
            }
            // Newer kinds of discovery news carry nothing a code needs.
            _ => {}
        }
    }
}

/// Serves every stream on one accepted connection until the peer closes it.
async fn serve_connection(
    incoming: iroh::endpoint::Incoming,
    handler: Arc<dyn PeerHandler>,
    gossip: iroh_gossip::net::Gossip,
) {
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::debug!(%error, "incoming group connection failed");
            return;
        }
    };
    if connection.alpn() == iroh_gossip::net::GOSSIP_ALPN {
        if let Err(error) = gossip.handle_connection(connection).await {
            tracing::debug!(%error, "gossip connection ended");
        }
        return;
    }
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

/// Hands every message on one joined topic to `handler` until the topic closes.
async fn listen_topic(
    mut receiver: iroh_gossip::api::GossipReceiver,
    handler: Arc<dyn sovra_core::group::GossipHandler>,
) {
    use n0_future::StreamExt as _;
    while let Some(event) = receiver.next().await {
        match event {
            Ok(iroh_gossip::api::Event::Received(message)) => {
                handler.received(message.content.to_vec()).await;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(%error, "broadcast topic closed");
                return;
            }
        }
    }
}

#[async_trait::async_trait]
impl GroupTransport for Mesh {
    async fn gossip_join(
        &self,
        topic: [u8; 32],
        peers: Vec<PeerAddress>,
        handler: Arc<dyn sovra_core::group::GossipHandler>,
        spawn: Spawn,
    ) {
        let mut ids = Vec::new();
        for peer in &peers {
            if let Ok(addr) = Self::target(peer) {
                ids.push(addr.id);
                self.known.add_endpoint_info(addr);
            }
        }
        let existing = self
            .topics
            .lock()
            .expect("topic table lock is never poisoned")
            .get(&topic)
            .cloned();
        if let Some(sender) = existing {
            if let Err(error) = sender.join_peers(ids).await {
                tracing::debug!(%error, "couldn't add peers to the broadcast topic");
            }
            return;
        }
        match self
            .gossip
            .subscribe(iroh_gossip::proto::TopicId::from_bytes(topic), ids)
            .await
        {
            Ok(joined) => {
                let (sender, receiver) = joined.split();
                self.topics
                    .lock()
                    .expect("topic table lock is never poisoned")
                    .insert(topic, sender);
                spawn(Box::pin(listen_topic(receiver, handler)));
            }
            Err(error) => tracing::debug!(%error, "couldn't join the broadcast topic"),
        }
    }

    async fn gossip_send(&self, topic: [u8; 32], message: Vec<u8>) {
        let sender = self
            .topics
            .lock()
            .expect("topic table lock is never poisoned")
            .get(&topic)
            .cloned();
        if let Some(sender) = sender
            && let Err(error) = sender.broadcast(message.into()).await
        {
            tracing::debug!(%error, "broadcast not sent");
        }
    }

    fn this_device(&self) -> DeviceId {
        DeviceId(*self.endpoint.id().as_bytes())
    }

    fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.secret.sign(message).to_bytes())
    }

    fn verifier(&self) -> Arc<dyn SignatureVerifier + Send + Sync> {
        Arc::new(Ed25519Verifier)
    }

    fn cipher(&self) -> Arc<dyn sovra_core::group::GroupCipher> {
        self.cipher.clone()
    }

    fn set_pairing_open(&self, hint: Option<u8>) {
        let tag = hint.and_then(|hint| format!("{PAIRING_TAG}{hint:x}").parse().ok());
        self.endpoint.set_user_data_for_address_lookup(tag);
    }

    async fn nearby_pairing(&self, hint: u8, listen_ms: u64) -> Vec<PeerAddress> {
        const POLL: std::time::Duration = std::time::Duration::from_millis(250);
        if self.nearby.is_none() {
            return Vec::new();
        }
        let wanted = format!("{PAIRING_TAG}{hint:x}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(listen_ms);
        loop {
            let found: Vec<PeerAddress> = self
                .heard
                .lock()
                .expect("nearby table lock is never poisoned")
                .values()
                .filter(|heard| heard.tag.as_deref() == Some(wanted.as_str()))
                .map(|heard| heard.address.clone())
                .collect();
            if !found.is_empty() || std::time::Instant::now() >= deadline {
                return found;
            }
            n0_future::time::sleep(POLL).await;
        }
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
        // Keep the nearby table current on the runtime's own task.
        if let Some(nearby) = self.nearby.clone() {
            let heard = self.heard.clone();
            let me = self.this_device();
            spawn(Box::pin(listen_nearby(nearby, heard, me)));
        }
        while let Some(incoming) = self.endpoint.accept().await {
            spawn(Box::pin(serve_connection(
                incoming,
                handler.clone(),
                self.gossip.clone(),
            )));
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
