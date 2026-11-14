//! Real child startup and ownership, without an installed model or external network.
use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn connection() -> ModelConnection {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut connection = ModelConnection::new("test", sovra_core::models::ProviderKind::Ollama);
    connection.base_url = format!("http://{}", listener.local_addr().unwrap());
    connection
}

#[tokio::test]
async fn missing_installation_is_actionable_and_remote_connections_do_not_launch() {
    let servers = LocalModelServers::new(vec![]);
    let mut connection = connection();
    assert!(
        servers
            .ensure_ready(&connection)
            .await
            .unwrap_err()
            .0
            .contains("Install")
    );
    connection.base_url = "http://remote.invalid:11434".into();
    servers.ensure_ready(&connection).await.unwrap();
    connection.base_url = "http://localhost:1234/v1".into();
    connection.kind = sovra_core::models::ProviderKind::OpenAiCompatible;
    servers.ensure_ready(&connection).await.unwrap();
    assert!(servers.children.lock().await.is_empty());
}

#[cfg(unix)]
fn program(directory: &std::path::Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = directory.join("ollama");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn starts_once_reuses_restarts_and_cleans_up_only_owned_servers() {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\'', "'\\''");
    let program = program(
        dir.path(),
        &format!("exec '{exe}' --ignored --exact local_models::tests::serve"),
    );
    let servers = LocalModelServers::new(vec![program]);
    let connection = connection();
    let (a, b) = tokio::join!(
        servers.ensure_ready(&connection),
        servers.ensure_ready(&connection)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(servers.children.lock().await.len(), 1);
    let address = sovra_providers::ollama_bind_address(&connection).unwrap();
    let pid = servers.children.lock().await[&address].id().unwrap();
    servers.ensure_ready(&connection).await.unwrap();
    assert_eq!(servers.children.lock().await[&address].id(), Some(pid));
    // An independent manager must reuse this server without taking ownership.
    let other = LocalModelServers::new(vec![]);
    other.ensure_ready(&connection).await.unwrap();
    assert!(other.children.lock().await.is_empty());
    drop(other);
    servers
        .children
        .lock()
        .await
        .get_mut(&address)
        .unwrap()
        .kill()
        .await
        .unwrap();
    servers.ensure_ready(&connection).await.unwrap();
    assert_ne!(servers.children.lock().await[&address].id(), Some(pid));
    drop(servers);
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::net::TcpStream::connect(&address).await.is_ok() {
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn early_exit_is_reported_without_waiting_for_the_chat_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let servers = LocalModelServers::new(vec![program(dir.path(), "exit 1")]);
    let result = tokio::time::timeout(Duration::from_secs(5), servers.ensure_ready(&connection()))
        .await
        .unwrap();
    assert!(result.unwrap_err().0.contains("exited"));
    assert!(servers.children.lock().await.is_empty());
}

#[tokio::test]
#[ignore = "child-process fixture, launched only by the ownership test"]
#[expect(
    clippy::disallowed_methods,
    reason = "isolated child fixture receives the bind address from its parent"
)]
async fn serve() {
    let address = std::env::var("OLLAMA_HOST").unwrap();
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    if std::env::var_os("SOVRA_TEST_START_DELAY").is_some() {
        sleep(Duration::from_secs(3)).await;
    }
    loop {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut input = [0; 4096];
        let count = stream.read(&mut input).await.unwrap_or(0);
        if count == 0 {
            continue;
        }
        let body = r#"{"version":"test"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn slow_initialization_can_outlast_one_health_probe() {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .replace('\'', "'\\''");
    let program = program(
        dir.path(),
        &format!(
            "export SOVRA_TEST_START_DELAY=1\nexec '{exe}' --ignored --exact local_models::tests::serve"
        ),
    );
    let servers = LocalModelServers::new(vec![program]);
    tokio::time::timeout(Duration::from_secs(8), servers.ensure_ready(&connection()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(servers.children.lock().await.len(), 1);
}
