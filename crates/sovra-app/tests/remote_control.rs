//! Production phone client, encrypted adapter and desktop actor together.
//! UI requests are acknowledged here; native rendering is covered in sovra-ui.
use sovra_core::{
    AgentConfig, Command, Event,
    development::UiRequest,
    remote_control::{ErrorCode, RemoteCommand, RemoteEvent, RemoteResponse, RemoteState},
};
use sovra_runtime::{Client, RemoteConfig, RemoteReferenceClient, RuntimeServices};
use std::{path::PathBuf, sync::Arc, time::Duration};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut suffix = [0; 16];
        getrandom::fill(&mut suffix).expect("random test directory");
        Self(std::env::temp_dir().join(format!("sovra-remote-network-{suffix:x?}")))
    }
    fn desktop(&self) -> Client {
        sovra_runtime::start_with_services(
            "remote-network-test",
            AgentConfig::default(),
            RuntimeServices {
                remote_control: Some(RemoteConfig {
                    directory: self.0.join("desktop"),
                    transport: Arc::new(|| {
                        Box::pin(async {
                            sovra_mesh::RemoteMesh::bind([71; 32], sovra_mesh::Reach::LocalOnly)
                                .await
                                .map(|endpoint| {
                                    Arc::new(endpoint)
                                        as Arc<dyn sovra_core::remote_control::RemoteEndpoint>
                                })
                                .map_err(|error| error.to_string())
                        })
                    }),
                    qr: Arc::new(sovra_whatsapp::qr_matrix),
                }),
                ..RuntimeServices::default()
            },
        )
        .expect("start desktop")
    }
    async fn phone(&self, key: u8) -> RemoteReferenceClient {
        let endpoint = sovra_mesh::RemoteMesh::bind([key; 32], sovra_mesh::Reach::LocalOnly)
            .await
            .expect("reference phone opens");
        RemoteReferenceClient::open(self.0.join("phone"), Arc::new(endpoint))
            .await
            .expect("reference phone opens")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn event(client: &Client) -> RemoteEvent {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::RemoteControl(event) =
                client.events().recv().await.expect("desktop event")
            {
                return *event;
            }
        }
    })
    .await
    .expect("bounded remote event")
}

async fn state(client: &Client) -> RemoteState {
    loop {
        if let RemoteEvent::State(state) = event(client).await {
            return state;
        }
    }
}

async fn acknowledge(client: &Client, expected: UiRequest) {
    loop {
        if let RemoteEvent::UiRequest {
            generation,
            request_id,
            request,
            ..
        } = event(client).await
        {
            assert_eq!(request, expected);
            client
                .send(Command::RemoteControl(RemoteCommand::UiReply {
                    generation,
                    request_id,
                    result: Ok(serde_json::json!({"revision":"observed", "input_delivered":true})),
                }))
                .expect("UI reply");
            return;
        }
    }
}

async fn inspect(phone: &mut RemoteReferenceClient, desktop: &Client) {
    let (response, ()) = tokio::join!(
        phone.control(UiRequest::Inspect {}, None),
        acknowledge(desktop, UiRequest::Inspect {}),
    );
    assert!(
        matches!(response, RemoteResponse::Result { .. }),
        "{response:?}"
    );
}

#[tokio::test]
async fn reference_pairing_does_not_accept_a_different_protocol_version() {
    use sovra_core::remote_control::{InvitationSecret, PairingTicket, RemoteEndpoint as _};
    let fixture = Fixture::new();
    let server = sovra_mesh::RemoteMesh::bind([74; 32], sovra_mesh::Reach::LocalOnly)
        .await
        .unwrap();
    let ticket = PairingTicket {
        version: 1,
        address: server.address(),
        secret: InvitationSecret([75; 32]),
    }
    .encode()
    .unwrap();
    let mut phone = fixture.phone(76).await;
    let (response, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(phone.pair(&ticket, "Version check".into()), async {
            let connection = server.accept().await.unwrap().complete().await.unwrap();
            let request = connection.receive().await.unwrap();
            request
                .reply
                .send(RemoteResponse::Ready {
                    version: 2,
                    generation: sovra_core::group::DeviceId([1; 32]),
                    session: sovra_core::group::DeviceId([2; 32]),
                    max_request_bytes: sovra_core::remote_control::MAX_REQUEST_BYTES,
                    max_response_bytes: sovra_core::remote_control::MAX_RESPONSE_BYTES,
                })
                .await
                .unwrap();
        },)
    })
    .await
    .unwrap();
    assert!(matches!(
        response,
        RemoteResponse::Error {
            code: ErrorCode::UnsupportedVersion,
            ..
        }
    ));
    assert!(!fixture.0.join("phone/desktop.json").exists());
    phone.close().await;
    server.close().await;
}

#[tokio::test]
async fn real_phone_pairs_reconnects_from_disk_and_never_replays_uncertain_input() {
    let fixture = Fixture::new();
    let desktop = fixture.desktop();
    assert!(state(&desktop).await.generation.is_none());
    desktop
        .send(Command::RemoteControl(RemoteCommand::Start))
        .unwrap();
    let link = state(&desktop).await.invitation.unwrap().encode().unwrap();
    let mut phone = fixture.phone(72).await;
    assert_eq!(
        phone.pair(&link, "Reference phone".into()).await,
        RemoteResponse::Paired {}
    );
    assert!(state(&desktop).await.invitation.is_none());
    // A lost acknowledgment can be recovered through Hello even after one-use
    // invitation consumption. No second grant or duplicate phone is created.
    assert_eq!(
        phone.pair(&link, "Reference phone".into()).await,
        RemoteResponse::Paired {}
    );
    inspect(&mut phone, &desktop).await;
    phone.close().await;
    let saved = std::fs::read_to_string(fixture.0.join("phone/desktop.json")).unwrap();
    assert!(!saved.contains("secret"));
    let mut phone = fixture.phone(72).await;
    inspect(&mut phone, &desktop).await;
    // A closed idle stream reconnects automatically and obtains new target IDs.
    tokio::time::sleep(Duration::from_secs(6)).await;
    inspect(&mut phone, &desktop).await;
    let request = UiRequest::Type {
        text: "test input".into(),
    };
    let (response, ()) = tokio::join!(
        phone.control(request.clone(), Some("observed".into())),
        acknowledge(&desktop, request),
    );
    assert!(matches!(response, RemoteResponse::Result { .. }));
    // Stop after input reaches the UI but before its reply. The client must not
    // reconnect and send that mutation again, even if its outcome is unknown.
    let request = UiRequest::Key {
        key: "enter".into(),
    };
    let (response, ()) = tokio::join!(
        phone.control(request.clone(), Some("observed".into())),
        async {
            loop {
                if let RemoteEvent::UiRequest {
                    request: actual, ..
                } = event(&desktop).await
                {
                    assert_eq!(actual, request);
                    desktop
                        .send(Command::RemoteControl(RemoteCommand::Stop))
                        .unwrap();
                    break;
                }
            }
        },
    );
    assert!(matches!(
        response,
        RemoteResponse::Error {
            code: ErrorCode::OutcomeUnknown,
            ..
        }
    ));
    assert!(state(&desktop).await.generation.is_none());
    assert!(matches!(
        phone
            .control(
                UiRequest::Type {
                    text: "must not send".into()
                },
                Some("observed".into())
            )
            .await,
        RemoteResponse::Error {
            code: ErrorCode::StaleSession,
            ..
        }
    ));
    phone.close().await;
    tokio::task::spawn_blocking(move || desktop.shutdown_blocking())
        .await
        .expect("reference phone opens")
        .unwrap();
}
