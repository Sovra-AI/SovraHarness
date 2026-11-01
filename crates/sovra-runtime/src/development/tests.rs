//! Exercise the real actor through Client and the authenticated local transport.
use super::*;
use serde_json::json;
use sovra_core::{AgentConfig, Command};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(
            std::env::temp_dir().join(format!("sovra-development-{}-{suffix}", std::process::id())),
        )
    }
    fn start(&self) -> crate::Client {
        crate::start_with_services(
            "test",
            AgentConfig::default(),
            crate::RuntimeServices {
                development: Some(DevelopmentConfig {
                    directory: self.0.clone(),
                    executable: PathBuf::from("/fixture/sovra"),
                }),
                ..crate::RuntimeServices::default()
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn next_development(events: &Receiver<Event>) -> DevelopmentEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::Development(event) = events.recv().await.unwrap() {
                return event;
            }
        }
    })
    .await
    .unwrap()
}

async fn state(events: &Receiver<Event>, enabled: bool) -> DevelopmentState {
    loop {
        if let DevelopmentEvent::State(state) = next_development(events).await
            && state.enabled == enabled
        {
            return state;
        }
    }
}

async fn stop(client: &crate::Client, events: &Receiver<Event>) {
    client.send(Command::Shutdown).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while events.recv().await.unwrap() != Event::Stopped {}
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn opt_in_ui_round_trip_revocation_and_restart_rotate_credentials() {
    let fixture = Fixture::new();
    let client = fixture.start();
    let events = client.events();
    assert!(!state(&events, false).await.enabled);
    assert!(
        endpoint::call(&fixture.0, UiRequest::Inspect {})
            .await
            .is_err()
    );
    client
        .send(Command::Development(DevelopmentCommand::SetEnabled(true)))
        .unwrap();
    let first = state(&events, true).await;
    let first_descriptor = tokio::fs::read(fixture.0.join("endpoint.json"))
        .await
        .unwrap();
    assert!(first.generation.is_some());
    assert!(first.error.is_none());
    let second_client = fixture.start();
    let second_events = second_client.events();
    assert!(state(&second_events, false).await.error.is_some());
    second_client
        .send(Command::Development(DevelopmentCommand::SetEnabled(true)))
        .unwrap();
    assert!(state(&second_events, false).await.error.is_some());
    assert!(
        endpoint::load_enabled(&fixture.0).await.unwrap(),
        "a failed second activation must preserve the owner's preference"
    );
    stop(&second_client, &second_events).await;

    let call = endpoint::call(&fixture.0, UiRequest::Inspect {});
    let reply = async {
        let DevelopmentEvent::UiRequest {
            generation,
            request_id,
            request,
            deadline,
        } = next_development(&events).await
        else {
            panic!("UI request")
        };
        assert_eq!(request, UiRequest::Inspect {});
        assert!(deadline > Instant::now());
        // A reply from an older activation must not satisfy a current request.
        client
            .send(Command::Development(DevelopmentCommand::UiReply {
                generation: "stale".into(),
                request_id,
                result: Ok(json!({"wrong":true})),
            }))
            .unwrap();
        client
            .send(Command::Development(DevelopmentCommand::UiReply {
                generation,
                request_id,
                result: Ok(json!({"elements":["native-control"]})),
            }))
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, reply);
    assert_eq!(result.unwrap(), json!({"elements":["native-control"]}));

    let call = endpoint::call(
        &fixture.0,
        UiRequest::Click {
            target: "native-control".into(),
        },
    );
    let disable = async {
        assert!(matches!(
            next_development(&events).await,
            DevelopmentEvent::UiRequest { .. }
        ));
        client
            .send(Command::Development(DevelopmentCommand::SetEnabled(false)))
            .unwrap();
        state(&events, false).await;
    };
    let (result, ()) = tokio::join!(call, disable);
    assert!(result.is_err());
    assert!(
        !tokio::fs::try_exists(fixture.0.join("endpoint.json"))
            .await
            .unwrap()
    );
    assert!(
        endpoint::call(&fixture.0, UiRequest::Inspect {})
            .await
            .is_err()
    );
    client
        .send(Command::Development(DevelopmentCommand::SetEnabled(true)))
        .unwrap();
    let second = state(&events, true).await;
    assert_ne!(first.generation, second.generation);
    assert_ne!(
        first_descriptor,
        tokio::fs::read(fixture.0.join("endpoint.json"))
            .await
            .unwrap()
    );
    stop(&client, &events).await;
    assert!(
        !tokio::fs::try_exists(fixture.0.join("endpoint.json"))
            .await
            .unwrap()
    );
    let client = fixture.start();
    let events = client.events();
    let restarted = state(&events, true).await;
    assert_ne!(second.generation, restarted.generation);
    client
        .send(Command::Development(DevelopmentCommand::SetEnabled(false)))
        .unwrap();
    state(&events, false).await;
    stop(&client, &events).await;
    let client = fixture.start();
    let events = client.events();
    state(&events, false).await;
    stop(&client, &events).await;
}

#[tokio::test]
async fn rejects_wrong_credentials_oversize_and_duplicate_owners() {
    let fixture = Fixture::new();
    let endpoint = endpoint::Active::open(&fixture.0).await.unwrap();
    assert!(endpoint::Active::open(&fixture.0).await.is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&fixture.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(fixture.0.join("endpoint.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    for payload in [
        b"{\"token\":\"wrong\",\"request\":{\"action\":\"inspect\"}}\n".to_vec(),
        vec![b'x'; MAX_FRAME + 1],
    ] {
        let address = endpoint.listener.local_addr().unwrap();
        let attacker = async {
            let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
            socket.write_all(&payload).await.unwrap();
            let mut reply = Vec::new();
            let _ = socket.read_to_end(&mut reply).await;
            assert_eq!(reply, Vec::<u8>::new());
        };
        let server = async {
            let (socket, _) = endpoint.listener.accept().await.unwrap();
            let (sender, receiver) = async_channel::bounded(1);
            assert!(
                endpoint::serve_connection(
                    socket,
                    &endpoint.descriptor.token,
                    endpoint.descriptor.generation.clone(),
                    sender
                )
                .await
                .is_err()
            );
            assert!(receiver.try_recv().is_err());
        };
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            tokio::join!(attacker, server);
        })
        .await
        .unwrap();
    }
    endpoint.close().await;
}

#[cfg(unix)]
#[tokio::test]
async fn refuses_public_directories_and_symlink_descriptors() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let fixture = Fixture::new();
    std::fs::create_dir(&fixture.0).unwrap();
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(endpoint::Active::open(&fixture.0).await.is_err());
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = fixture.0.join("target.json");
    std::fs::write(&target, "{}").unwrap();
    symlink(&target, fixture.0.join("endpoint.json")).unwrap();
    assert!(
        endpoint::call(&fixture.0, UiRequest::Inspect {})
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "{}");
}
