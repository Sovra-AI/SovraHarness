//! Drive the real actor through Client with an in-process authenticated transport.
use super::*;
use sovra_core::{
    AgentConfig, Command,
    development::UiRequest,
    group::{PeerAddress, TransportError},
    remote_control::{IncomingRequest, RemoteHandshake, RemoteReply},
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
struct Endpoint {
    incoming: Receiver<Box<dyn RemoteHandshake>>,
    sender: Sender<Box<dyn RemoteHandshake>>,
    closed: AtomicBool,
}

#[derive(Debug)]
struct Connection {
    peer: DeviceId,
    requests: Receiver<IncomingRequest>,
}

#[derive(Debug)]
struct Handshake(Arc<Connection>);

#[derive(Debug)]
struct Reply(oneshot::Sender<RemoteResponse>);

#[derive(Clone)]
struct Phone(Sender<IncomingRequest>);

impl Endpoint {
    async fn phone(&self, peer: DeviceId) -> Phone {
        let (sender, requests) = async_channel::bounded(8);
        self.sender
            .send(Box::new(Handshake(Arc::new(Connection { peer, requests }))))
            .await
            .unwrap();
        Phone(sender)
    }
}

impl Phone {
    async fn call(&self, operation: RemoteOperation) -> RemoteResponse {
        let (reply, response) = oneshot::channel();
        self.0
            .send(IncomingRequest {
                request: RemoteRequest {
                    version: 1,
                    operation,
                },
                reply: Box::new(Reply(reply)),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), response)
            .await
            .unwrap()
            .unwrap()
    }
}

#[async_trait::async_trait]
impl RemoteEndpoint for Endpoint {
    fn address(&self) -> PeerAddress {
        PeerAddress {
            device: DeviceId([7; 32]),
            relay: None,
            addrs: vec!["127.0.0.1:1234".into()],
        }
    }
    async fn accept(&self) -> Result<Box<dyn RemoteHandshake>, TransportError> {
        self.incoming
            .recv()
            .await
            .map_err(|_| TransportError::Closed)
    }
    async fn connect(&self, _: &PeerAddress) -> Result<Arc<dyn RemoteConnection>, TransportError> {
        Err(TransportError::Closed)
    }
    async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.incoming.close();
    }
}

#[async_trait::async_trait]
impl RemoteHandshake for Handshake {
    async fn complete(self: Box<Self>) -> Result<Arc<dyn RemoteConnection>, TransportError> {
        Ok(self.0)
    }
}

#[async_trait::async_trait]
impl RemoteConnection for Connection {
    fn peer(&self) -> DeviceId {
        self.peer
    }
    async fn receive(&self) -> Result<IncomingRequest, TransportError> {
        self.requests
            .recv()
            .await
            .map_err(|_| TransportError::Closed)
    }
    async fn request(&self, _: &RemoteRequest) -> Result<RemoteResponse, TransportError> {
        Err(TransportError::Closed)
    }
    fn close(&self) {
        self.requests.close();
    }
}

#[async_trait::async_trait]
impl RemoteReply for Reply {
    async fn send(self: Box<Self>, response: RemoteResponse) -> Result<(), TransportError> {
        self.0.send(response).map_err(|_| TransportError::Closed)
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("sovra-phone-{}", nonce().unwrap().to_hex())))
    }
    fn start(&self) -> (crate::Client, Receiver<Arc<Endpoint>>) {
        let (output, endpoints) = async_channel::bounded(8);
        let client = crate::start_with_services(
            "test",
            AgentConfig::default(),
            crate::RuntimeServices {
                remote_control: Some(RemoteConfig {
                    directory: self.0.clone(),
                    qr: Arc::new(|_| Vec::new()),
                    transport: Arc::new(move || {
                        let output = output.clone();
                        Box::pin(async move {
                            let (sender, incoming) = async_channel::bounded(16);
                            let endpoint = Arc::new(Endpoint {
                                incoming,
                                sender,
                                closed: AtomicBool::new(false),
                            });
                            output
                                .send(endpoint.clone())
                                .await
                                .map_err(|_| "test observer closed".to_owned())?;
                            Ok(endpoint as Arc<dyn RemoteEndpoint>)
                        })
                    }),
                }),
                ..crate::RuntimeServices::default()
            },
        )
        .unwrap();
        (client, endpoints)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn event(client: &crate::Client) -> RemoteEvent {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Event::RemoteControl(event) = client.events().recv().await.unwrap() {
                return *event;
            }
        }
    })
    .await
    .unwrap()
}

async fn state(client: &crate::Client) -> RemoteState {
    loop {
        if let RemoteEvent::State(state) = event(client).await {
            return state;
        }
    }
}

fn send(client: &crate::Client, command: RemoteCommand) {
    client.send(Command::RemoteControl(command)).unwrap();
}

async fn stop(client: &crate::Client, fixture: &Fixture) {
    client.send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(fixture.0.join("owner.lock"))
                && file.try_lock().is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn pair_control_stop_reconnect_and_restart_through_client() {
    let fixture = Fixture::new();
    let (client, endpoints) = fixture.start();
    assert_eq!(state(&client).await.generation, None);
    send(&client, RemoteCommand::Start);
    let first = state(&client).await;
    let invitation = first.invitation.clone().unwrap();
    send(&client, RemoteCommand::Start);
    assert_eq!(state(&client).await, first);
    let endpoint = endpoints.recv().await.unwrap();
    let peer = DeviceId([3; 32]);
    let phone = endpoint.phone(peer).await;
    assert!(matches!(
        phone.call(RemoteOperation::Hello {}).await,
        RemoteResponse::Error {
            code: ErrorCode::NotPaired,
            ..
        }
    ));
    assert_eq!(
        phone
            .call(RemoteOperation::Pair {
                secret: invitation.secret,
                name: "Test phone".into()
            })
            .await,
        RemoteResponse::Paired {}
    );
    let paired = state(&client).await;
    assert_eq!(paired.phones.len(), 1);
    assert!(paired.invitation.is_none());
    let RemoteResponse::Ready {
        generation,
        session,
        ..
    } = phone.call(RemoteOperation::Hello {}).await
    else {
        panic!("ready")
    };
    assert_eq!(state(&client).await.connected_phone, Some(peer));
    let call = tokio::runtime::Handle::current().spawn({
        let phone = phone.clone();
        async move {
            phone
                .call(RemoteOperation::Control {
                    generation,
                    session,
                    sequence: 1,
                    revision: None,
                    request: UiRequest::Inspect {},
                })
                .await
        }
    });
    let RemoteEvent::UiRequest {
        request_id,
        phone: observed,
        ..
    } = event(&client).await
    else {
        panic!("UI request")
    };
    assert_eq!(observed, peer);
    send(
        &client,
        RemoteCommand::UiReply {
            generation,
            request_id,
            result: Ok(serde_json::json!({"elements":[]})),
        },
    );
    assert!(matches!(call.await.unwrap(), RemoteResponse::Result { .. }));
    assert!(matches!(
        phone
            .call(RemoteOperation::Control {
                generation,
                session,
                sequence: 1,
                revision: None,
                request: UiRequest::Inspect {}
            })
            .await,
        RemoteResponse::Error {
            code: ErrorCode::StaleSession,
            ..
        }
    ));
    send(&client, RemoteCommand::PairAnother);
    assert!(state(&client).await.invitation.is_some());
    assert!(matches!(
        phone
            .call(RemoteOperation::Control {
                generation,
                session,
                sequence: 2,
                revision: None,
                request: UiRequest::Screenshot {},
            })
            .await,
        RemoteResponse::Error {
            code: ErrorCode::Busy,
            ..
        }
    ));
    send(&client, RemoteCommand::Stop);
    assert_eq!(state(&client).await.generation, None);
    assert!(endpoint.closed.load(Ordering::SeqCst));
    send(&client, RemoteCommand::Start);
    let enabled = state(&client).await;
    assert!(enabled.invitation.is_none());
    assert_ne!(enabled.generation, Some(generation));
    let endpoint = endpoints.recv().await.unwrap();
    let phone = endpoint.phone(peer).await;
    assert!(matches!(
        phone.call(RemoteOperation::Hello {}).await,
        RemoteResponse::Ready { .. }
    ));
    assert_eq!(state(&client).await.connected_phone, Some(peer));
    stop(&client, &fixture).await;
    let (client, endpoints) = fixture.start();
    let restored = state(&client).await;
    assert_eq!(restored.generation, None);
    assert_eq!(restored.phones, paired.phones);
    send(&client, RemoteCommand::Start);
    assert!(state(&client).await.invitation.is_none());
    let endpoint = endpoints.recv().await.unwrap();
    let phone = endpoint.phone(peer).await;
    assert!(matches!(
        phone.call(RemoteOperation::Hello {}).await,
        RemoteResponse::Ready { .. }
    ));
    state(&client).await;
    send(&client, RemoteCommand::Forget { device: peer });
    let forgotten = state(&client).await;
    assert_eq!(forgotten.phones, []);
    assert_eq!(forgotten.connected_phone, None);
    let phone = endpoint.phone(peer).await;
    assert!(matches!(
        phone.call(RemoteOperation::Hello {}).await,
        RemoteResponse::Error {
            code: ErrorCode::NotPaired,
            ..
        }
    ));
    stop(&client, &fixture).await;
}

#[tokio::test]
async fn failed_pair_save_does_not_grant_access_and_failed_forget_stops_access() {
    let fixture = Fixture::new();
    let (client, endpoints) = fixture.start();
    state(&client).await;
    send(&client, RemoteCommand::Start);
    let secret = state(&client).await.invitation.unwrap().secret;
    let endpoint = endpoints.recv().await.unwrap();
    let peer = DeviceId([4; 32]);
    let phone = endpoint.phone(peer).await;
    std::fs::create_dir(fixture.0.join("phones.json")).unwrap();
    assert!(matches!(
        phone
            .call(RemoteOperation::Pair {
                secret: secret.clone(),
                name: "Test".into()
            })
            .await,
        RemoteResponse::Error {
            code: ErrorCode::StorageUnavailable,
            ..
        }
    ));
    assert!(matches!(
        phone.call(RemoteOperation::Hello {}).await,
        RemoteResponse::Error {
            code: ErrorCode::NotPaired,
            ..
        }
    ));
    std::fs::remove_dir(fixture.0.join("phones.json")).unwrap();
    assert_eq!(
        phone
            .call(RemoteOperation::Pair {
                secret,
                name: "Test".into()
            })
            .await,
        RemoteResponse::Paired {}
    );
    state(&client).await;
    std::fs::remove_file(fixture.0.join("phones.json")).unwrap();
    std::fs::create_dir(fixture.0.join("phones.json")).unwrap();
    send(&client, RemoteCommand::Forget { device: peer });
    let failed = state(&client).await;
    assert_eq!(failed.generation, None);
    assert_eq!(failed.phones.len(), 1);
    assert!(failed.error.unwrap().contains("not forgotten"));
    assert!(endpoint.closed.load(Ordering::SeqCst));
    stop(&client, &fixture).await;
}

#[tokio::test]
async fn corrupt_storage_and_second_owner_fail_closed() {
    let fixture = Fixture::new();
    crate::private_storage::write_private(&fixture.0, "phones.json", &"corrupt")
        .await
        .unwrap();
    let (client, endpoints) = fixture.start();
    assert!(state(&client).await.error.is_some());
    send(&client, RemoteCommand::Start);
    assert_eq!(state(&client).await.generation, None);
    assert!(endpoints.is_empty());
    client.send(Command::Shutdown).unwrap();
    // The lock is released by a failed load; repair explicitly before the next start.
    crate::private_storage::write_private(
        &fixture.0,
        "phones.json",
        &Vec::<sovra_core::remote_control::PairedPhone>::new(),
    )
    .await
    .unwrap();
    let (owner, _) = fixture.start();
    assert!(state(&owner).await.error.is_none());
    let (second, endpoints) = fixture.start();
    assert!(state(&second).await.error.is_some());
    send(&second, RemoteCommand::Start);
    assert_eq!(state(&second).await.generation, None);
    assert!(endpoints.is_empty());
    second.send(Command::Shutdown).unwrap();
    stop(&owner, &fixture).await;
}

#[tokio::test]
async fn a_late_connected_message_cannot_retain_a_finished_connection() {
    let (events, _) = async_channel::unbounded();
    let mut actor = Actor::new(None, events).await;
    let generation = DeviceId([8; 32]);
    actor.state.generation = Some(generation);
    let (_, requests) = async_channel::bounded(1);
    let connection = Arc::new(Connection {
        peer: DeviceId([9; 32]),
        requests,
    });
    actor.record_connection(generation, 1, connection.clone());
    assert!(connection.requests.is_closed());
    assert_eq!(actor.connections.len(), 0);
}
