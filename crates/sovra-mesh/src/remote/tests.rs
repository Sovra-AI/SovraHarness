use std::time::Duration;

use sovra_core::remote_control::{InvitationSecret, RemoteOperation};

use super::*;

#[tokio::test]
#[ignore = "requires Internet access to the public Iroh relay; run explicitly"]
async fn remote_protocol_crosses_a_public_relay_with_direct_networking_disabled() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let desktop = RemoteMesh::bind([61; 32], Reach::Internet).await.unwrap();
        let phone = Endpoint::builder(presets::N0)
            .secret_key(SecretKey::from_bytes(&[62; 32]))
            .alpns(vec![REMOTE_ALPN.to_vec()])
            .clear_ip_transports()
            .bind()
            .await
            .unwrap();
        desktop.endpoint.online().await;
        phone.online().await;
        assert_eq!(phone.bound_sockets(), [] as [std::net::SocketAddr; 0]);
        let (server, raw) = tokio::join!(
            async { desktop.accept().await.unwrap().complete().await.unwrap() },
            phone.connect(desktop.endpoint.addr(), REMOTE_ALPN),
        );
        let raw = raw.unwrap();
        let client = connection(raw.clone()).unwrap();
        let request = RemoteRequest {
            version: 1,
            operation: RemoteOperation::Hello {},
        };
        let response = RemoteResponse::Result {
            value: serde_json::json!({"text": "relay".repeat(20_000)}),
        };
        let ((), actual) = tokio::join!(
            async {
                let incoming = server.receive().await.unwrap();
                assert_eq!(incoming.request, request);
                incoming.reply.send(response.clone()).await.unwrap();
            },
            client.request(&request),
        );
        assert_eq!(actual.unwrap(), response);
        assert!(raw.paths().iter().any(|path| path.is_relay()));
        assert!(!raw.paths().iter().any(|path| path.is_ip()));
        client.close();
        desktop.close().await;
        phone.close().await;
    })
    .await
    .expect("relay roundtrip within 45 seconds");
}

async fn connected(
    desktop: &RemoteMesh,
    phone: &RemoteMesh,
) -> (Arc<dyn RemoteConnection>, Arc<dyn RemoteConnection>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let address = desktop.address();
        let (server, client) = tokio::join!(
            async { desktop.accept().await.unwrap().complete().await.unwrap() },
            phone.connect(&address),
        );
        (server, client.unwrap())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn authenticated_phone_identity_and_multiple_bounded_exchanges() {
    let desktop = RemoteMesh::bind([31; 32], Reach::LocalOnly).await.unwrap();
    let phone = RemoteMesh::bind([32; 32], Reach::LocalOnly).await.unwrap();
    let (server, client) = connected(&desktop, &phone).await;
    assert_eq!(server.peer(), phone.address().device);
    assert_eq!(client.peer(), desktop.address().device);
    for operation in [
        RemoteOperation::Pair {
            secret: InvitationSecret([8; 32]),
            name: "My phone".into(),
        },
        RemoteOperation::Hello {},
    ] {
        let request = RemoteRequest {
            version: 1,
            operation,
        };
        let response = RemoteResponse::Result {
            value: serde_json::json!({"text": "x".repeat(100_000)}),
        };
        let actual = tokio::time::timeout(Duration::from_secs(10), async {
            let ((), actual) = tokio::join!(
                async {
                    let incoming = server.receive().await.unwrap();
                    assert_eq!(incoming.request, request);
                    incoming.reply.send(response.clone()).await.unwrap();
                },
                client.request(&request)
            );
            actual.unwrap()
        })
        .await
        .unwrap();
        assert_eq!(actual, response);
    }
    server.close();
    assert!(
        tokio::time::timeout(
            Duration::from_secs(5),
            client.request(&RemoteRequest {
                version: 1,
                operation: RemoteOperation::Hello {}
            })
        )
        .await
        .unwrap()
        .is_err()
    );
    desktop.close().await;
    phone.close().await;
}

#[tokio::test]
async fn wrong_protocol_never_becomes_a_remote_connection() {
    let desktop = RemoteMesh::bind([33; 32], Reach::LocalOnly).await.unwrap();
    let phone = RemoteMesh::bind([34; 32], Reach::LocalOnly).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        let (server, client) = tokio::join!(
            async { desktop.accept().await.unwrap().complete().await },
            phone
                .endpoint
                .connect(Mesh::target(&desktop.address()).unwrap(), b"sovra/groups/1"),
        );
        assert!(server.is_err());
        assert!(client.is_err());
    })
    .await
    .unwrap();
    desktop.close().await;
    phone.close().await;
}

#[tokio::test]
async fn malformed_and_oversized_frames_never_reach_the_handler() {
    let desktop = RemoteMesh::bind([35; 32], Reach::LocalOnly).await.unwrap();
    let phone = RemoteMesh::bind([36; 32], Reach::LocalOnly).await.unwrap();
    for bytes in [b"not json".to_vec(), vec![b'x'; MAX_REQUEST_BYTES + 1]] {
        tokio::time::timeout(Duration::from_secs(10), async {
            let (server, raw) = tokio::join!(
                async { desktop.accept().await.unwrap().complete().await.unwrap() },
                phone
                    .endpoint
                    .connect(Mesh::target(&desktop.address()).unwrap(), REMOTE_ALPN),
            );
            let raw = raw.unwrap();
            let (result, ()) = tokio::join!(server.receive(), async {
                let (mut send, mut recv) = raw.open_bi().await.unwrap();
                send.write_all(&bytes).await.unwrap();
                send.finish().unwrap();
                let response = recv.read_to_end(MAX_RESPONSE_BYTES).await;
                if bytes.len() <= MAX_REQUEST_BYTES {
                    assert!(matches!(
                        serde_json::from_slice::<RemoteResponse>(&response.unwrap()).unwrap(),
                        RemoteResponse::Error {
                            code: ErrorCode::InvalidRequest,
                            ..
                        }
                    ));
                }
            });
            assert!(result.is_err());
            raw.close(0u32.into(), b"test finished");
        })
        .await
        .unwrap();
    }
    desktop.close().await;
    phone.close().await;
}

#[tokio::test]
async fn changed_desktop_key_cannot_impersonate_a_saved_desktop() {
    let desktop = RemoteMesh::bind([37; 32], Reach::LocalOnly).await.unwrap();
    let phone = RemoteMesh::bind([38; 32], Reach::LocalOnly).await.unwrap();
    let mut address = desktop.address();
    address.device = Mesh::device_for(&[39; 32]);
    let attempt = tokio::time::timeout(Duration::from_secs(3), async {
        let (server, client) = tokio::join!(
            async { desktop.accept().await.unwrap().complete().await },
            phone.connect(&address),
        );
        assert!(server.is_err());
        client
    })
    .await;
    assert!(!matches!(attempt, Ok(Ok(_))));
    desktop.close().await;
    phone.close().await;
}
