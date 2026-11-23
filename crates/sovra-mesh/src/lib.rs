//! Peer-to-peer adapter for groups: the only crate that names `iroh`.
//!
//! Owns this computer's device key (an Ed25519 secret that never leaves the
//! adapter), dials other computers by their public key over QUIC with hole punching
//! and an encrypted relay fallback, and carries [`PeerRequest`]s one per stream.
//! One connection per computer stays open and is reused: across the internet the
//! first packets travel through a relay while a direct way is found, so a fresh
//! connection per message would keep every message on the slow road.
//! It decides nothing: every rule (who may join, sync or push, how history folds)
//! lives in `sovra_core::group`, and every task runs on the spawner the runtime
//! hands in, so the runtime owns cancellation. This crate never names `tokio`.

mod cipher;
mod remote;
mod store;
mod swarm;

use std::str::FromStr as _;
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayUrl, SecretKey, TransportAddr};
use sovra_core::group::{
    DeviceId, GroupTransport, MAX_PEER_MESSAGE_BYTES, PEER_ALPN, PEER_ALPN_DEFLATE, PeerAddress,
    PeerHandler, PeerRequest, PeerResponse, Reachability, Route, Signature, SignatureVerifier,
    Spawn, TransportError,
};

pub use remote::RemoteMesh;
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
    /// Like [`Self::Internet`], but never a direct connection: every packet goes
    /// through a relay, as on a network that blocks everything else. For checking
    /// that a group still works on such a network; slower, so not for daily use.
    RelayOnly,
}

/// How long a computer may take to get onto the internet after starting before
/// it is called offline instead of "getting connected".
const CONNECTING_GRACE: std::time::Duration = std::time::Duration::from_secs(20);

/// The handler and task spawner [`GroupTransport::serve`] was started with.
#[derive(Clone, Default)]
struct Serving(Arc<std::sync::Mutex<Option<Server>>>);

/// Who answers requests, and where their tasks run.
type Server = (Arc<dyn PeerHandler>, Spawn);

impl std::fmt::Debug for Serving {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Serving")
    }
}

impl Serving {
    fn get(&self) -> Option<Server> {
        self.0
            .lock()
            .expect("serving lock is never poisoned")
            .clone()
    }

    fn set(&self, serving: Option<Server>) {
        *self.0.lock().expect("serving lock is never poisoned") = serving;
    }
}

/// One open connection per computer, reused for every request to it.
type Pool = Arc<std::sync::Mutex<std::collections::BTreeMap<DeviceId, Pooled>>>;

/// A connection younger than this that is replaced in the pool is left to finish
/// what it carries: two computers that dial each other at the same moment each
/// end up using the other's connection, and neither is cut off.
const SETTLED_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// Makes the connection `from` just opened the one this computer uses for its
/// own requests to `from`. Requests travel both ways on one connection, so there
/// is nothing to dial; and when a computer comes back after a crash, a closed
/// lid or a quick restart, with no goodbye, its first word arrives on a new
/// connection and the answer goes straight back on it. Dialling it instead can
/// hang for half a minute, because the connections it left behind go on looking
/// open and packets keep going to where it used to be.
///
/// The connection it replaces, if it has been in use for a while, is closed:
/// whatever was waiting on it was waiting for a program that is gone, and is
/// sent again on the new one.
fn adopt(pool: &Pool, from: DeviceId, connection: &iroh::endpoint::Connection) {
    let replaced = pool
        .lock()
        .expect("connection pool lock is never poisoned")
        .insert(
            from,
            Pooled {
                connection: connection.clone(),
                busy: Arc::default(),
                opened: std::time::Instant::now(),
            },
        );
    if let Some(replaced) = replaced
        && replaced.connection.stable_id() != connection.stable_id()
        && replaced.opened.elapsed() >= SETTLED_AFTER
    {
        replaced.connection.close(0u32.into(), b"replaced");
    }
}

/// A pooled connection and how many requests are travelling on it right now.
#[derive(Debug, Clone)]
struct Pooled {
    connection: iroh::endpoint::Connection,
    busy: Arc<std::sync::atomic::AtomicUsize>,
    opened: std::time::Instant,
}

/// Counts one request on a pooled connection. When the request ends without an
/// answer (it failed, or the runtime stopped waiting and dropped it) and nothing
/// else is travelling on the connection, the connection is closed and forgotten:
/// a computer that restarted is then dialled afresh at once, not after its old
/// connection times out. While anything else is on the connection it is left
/// alone, so a large document is never cut off by a small question beside it
/// running out of time.
struct InFlight<'a> {
    pool: &'a Pool,
    device: DeviceId,
    pooled: Pooled,
    answered: bool,
}

impl<'a> InFlight<'a> {
    fn start(pool: &'a Pool, device: DeviceId, pooled: Pooled) -> Self {
        pooled
            .busy
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self {
            pool,
            device,
            pooled,
            answered: false,
        }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let left = self
            .pooled
            .busy
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
            .saturating_sub(1);
        if self.answered || left > 0 {
            return;
        }
        let mut pool = self
            .pool
            .lock()
            .expect("connection pool lock is never poisoned");
        if pool.get(&self.device).is_some_and(|pooled| {
            pooled.connection.stable_id() == self.pooled.connection.stable_id()
        }) {
            pool.remove(&self.device);
        }
        drop(pool);
        self.pooled.connection.close(0u32.into(), b"gave up");
    }
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
    /// Open connections to other computers, reused request after request.
    pool: Pool,
    /// Who answers requests, once [`GroupTransport::serve`] has started: a
    /// connection this computer opens carries the other computer's requests too.
    serving: Serving,
    reach: Reach,
    started: std::time::Instant,
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
            Reach::RelayOnly => Endpoint::builder(presets::N0).clear_ip_transports(),
        };
        let endpoint = builder
            .secret_key(secret.clone())
            .alpns(vec![
                // The accepting side chooses: deflated frames when the caller
                // knows them, plain ones for a computer on an older Sovra.
                PEER_ALPN_DEFLATE.to_vec(),
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
            pool: Arc::default(),
            serving: Serving::default(),
            reach,
            started: std::time::Instant::now(),
        })
    }

    /// The open connection to `to`, or a new one. A pooled connection that has
    /// closed since (the other computer went away) is replaced.
    async fn connection(&self, to: &PeerAddress) -> Result<Pooled, TransportError> {
        let pooled = self
            .pool
            .lock()
            .expect("connection pool lock is never poisoned")
            .get(&to.device)
            .cloned();
        if let Some(pooled) = pooled
            && pooled.connection.close_reason().is_none()
        {
            return Ok(pooled);
        }
        let connection = self
            .endpoint
            .connect_with_opts(
                Self::target(to)?,
                PEER_ALPN_DEFLATE,
                iroh::endpoint::ConnectOptions::new()
                    .with_additional_alpns(vec![PEER_ALPN.to_vec()]),
            )
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        // Both computers send requests on this one connection (only computers
        // that deflate know to), so answer the other one's here too.
        if let Some((handler, spawn)) = self.serving.get()
            && connection.alpn() == PEER_ALPN_DEFLATE
        {
            let tasks = spawn.clone();
            spawn(Box::pin(serve_requests(
                connection.clone(),
                handler,
                tasks,
                to.device,
                true,
            )));
        }
        let pooled = Pooled {
            connection,
            busy: Arc::default(),
            opened: std::time::Instant::now(),
        };
        // Two requests may have dialled at once; the last one in is kept and the
        // other closes once its request is done and it is dropped.
        self.pool
            .lock()
            .expect("connection pool lock is never poisoned")
            .insert(to.device, pooled.clone());
        Ok(pooled)
    }

    /// One request and its answer on its own stream of `connection`.
    async fn exchange(
        connection: &iroh::endpoint::Connection,
        request: &PeerRequest,
    ) -> Result<PeerResponse, TransportError> {
        fn unreachable(error: impl std::fmt::Display) -> TransportError {
            TransportError::Unreachable(error.to_string())
        }
        let deflated = connection.alpn() == PEER_ALPN_DEFLATE;
        let (mut send, mut recv) = connection.open_bi().await.map_err(unreachable)?;
        send.write_all(&frame(request, deflated))
            .await
            .map_err(unreachable)?;
        send.finish().map_err(unreachable)?;
        let bytes = recv
            .read_to_end(MAX_PEER_MESSAGE_BYTES)
            .await
            .map_err(|error| TransportError::Protocol(error.to_string()))?;
        if bytes.is_empty() {
            // The stream ended with no answer on it: whatever was answering
            // stopped (a computer shutting down or starting again).
            return Err(TransportError::Unreachable("no answer".into()));
        }
        unframe(&bytes, deflated)
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

/// One message as it travels: JSON, deflated when both computers agreed to it.
fn frame<T: serde::Serialize>(value: &T, deflated: bool) -> Vec<u8> {
    use std::io::Write as _;
    let json = encode(value);
    if !deflated {
        return json;
    }
    // The default level, not the fastest: some deflate backends skip the coding
    // step at their fastest level, and that step is all that shrinks hex.
    let mut packer = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    packer
        .write_all(&json)
        .and_then(|()| packer.finish())
        .expect("writing to memory cannot fail")
}

/// Reads one message. A deflated frame is never unpacked beyond the largest
/// message allowed, however small it was made to look on the wire.
fn unframe<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    deflated: bool,
) -> Result<T, TransportError> {
    use std::io::Read as _;
    if !deflated {
        return decode(bytes);
    }
    let limit = u64::try_from(MAX_PEER_MESSAGE_BYTES).unwrap_or(u64::MAX);
    let mut json = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .take(limit.saturating_add(1))
        .read_to_end(&mut json)
        .map_err(|error| TransportError::Protocol(error.to_string()))?;
    if json.len() > MAX_PEER_MESSAGE_BYTES {
        return Err(TransportError::Protocol("message too large".into()));
    }
    decode(&json)
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

/// Accepts every request on one connection until the other computer closes it.
async fn serve_connection(
    incoming: iroh::endpoint::Incoming,
    handler: Arc<dyn PeerHandler>,
    gossip: iroh_gossip::net::Gossip,
    spawn: Spawn,
    pool: Pool,
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
    let deflated = connection.alpn() == PEER_ALPN_DEFLATE;
    if deflated {
        adopt(&pool, from, &connection);
    }
    serve_requests(connection, handler, spawn, from, deflated).await;
}

/// Answers every request arriving on `connection` until it closes. Each request
/// gets its own task: everything between two computers shares one connection,
/// and a small question must not wait behind a large document, nor may one
/// abandoned request end the connection for the rest.
async fn serve_requests(
    connection: iroh::endpoint::Connection,
    handler: Arc<dyn PeerHandler>,
    spawn: Spawn,
    from: DeviceId,
    deflated: bool,
) {
    while let Ok((send, recv)) = connection.accept_bi().await {
        spawn(Box::pin(serve_request(
            send,
            recv,
            handler.clone(),
            from,
            deflated,
        )));
    }
}

/// Answers one request on its own stream.
async fn serve_request(
    mut send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    handler: Arc<dyn PeerHandler>,
    from: DeviceId,
    deflated: bool,
) {
    let response = match recv.read_to_end(MAX_PEER_MESSAGE_BYTES).await {
        Ok(bytes) => match unframe::<PeerRequest>(&bytes, deflated) {
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
    if send.write_all(&frame(&response, deflated)).await.is_err() || send.finish().is_err() {
        tracing::debug!("group answer could not be sent");
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
        peer_address(&self.endpoint)
    }

    async fn serve(&self, handler: Arc<dyn PeerHandler>, spawn: Spawn) {
        self.serving.set(Some((handler.clone(), spawn.clone())));
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
                spawn.clone(),
                self.pool.clone(),
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
        // A pooled connection can die between two requests before this side has
        // noticed; one fresh connection settles whether the other computer is
        // really gone.
        let mut last = TransportError::Unreachable("no connection".into());
        for _attempt in 0..2 {
            let pooled = self.connection(to).await?;
            let mut flight = InFlight::start(&self.pool, to.device, pooled.clone());
            match Self::exchange(&pooled.connection, &request).await {
                Ok(response) => {
                    flight.answered = true;
                    return Ok(response);
                }
                Err(error) => {
                    // Worth one more try on a connection that is open now: the
                    // old one closed, or went quiet without an answer.
                    let again = pooled.connection.close_reason().is_some()
                        || matches!(error, TransportError::Unreachable(_));
                    last = error;
                    drop(flight);
                    if !again {
                        break;
                    }
                }
            }
        }
        Err(last)
    }

    fn reachability(&self) -> Reachability {
        use iroh::Watcher as _;
        if matches!(self.reach, Reach::LocalOnly) {
            return Reachability::SameNetworkOnly;
        }
        if self
            .endpoint
            .home_relay_status()
            .get()
            .iter()
            .any(iroh::endpoint::RelayStatus::is_connected)
        {
            Reachability::Anywhere
        } else if self.started.elapsed() < CONNECTING_GRACE {
            Reachability::Connecting
        } else {
            Reachability::Offline
        }
    }

    async fn wait_until_reachable(&self, wait_ms: u64) {
        if matches!(self.reach, Reach::LocalOnly) {
            return;
        }
        let _ = n0_future::time::timeout(
            std::time::Duration::from_millis(wait_ms),
            self.endpoint.online(),
        )
        .await;
    }

    fn route_to(&self, device: &DeviceId) -> Option<Route> {
        let connection = self
            .pool
            .lock()
            .expect("connection pool lock is never poisoned")
            .get(device)
            .cloned()?
            .connection;
        if connection.close_reason().is_some() {
            return None;
        }
        let paths = connection.paths();
        let selected = paths.iter().find(iroh::endpoint::Path::is_selected)?;
        Some(if selected.is_relay() {
            Route::Relayed
        } else {
            Route::Direct
        })
    }

    async fn close(&self) {
        let open = std::mem::take(
            &mut *self
                .pool
                .lock()
                .expect("connection pool lock is never poisoned"),
        );
        for pooled in open.into_values() {
            pooled.connection.close(0u32.into(), b"done");
        }
        self.serving.set(None);
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests;

fn peer_address(endpoint: &Endpoint) -> PeerAddress {
    let addr = endpoint.addr();
    PeerAddress {
        device: DeviceId(*endpoint.id().as_bytes()),
        relay: addr.relay_urls().next().map(ToString::to_string),
        addrs: addr.ip_addrs().map(ToString::to_string).collect(),
    }
}
