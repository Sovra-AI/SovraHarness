//! Exercises real SSH framing without a Pi or an external SSH installation.
use super::*;
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server::{self, Session},
};
use std::sync::atomic::{AtomicUsize, Ordering};

const PASSWORD: &str = " test password with spaces ";
const REPORT: &str = r#"{"model":"Raspberry Pi 5 Model B Rev 1.0"}"#;

struct FakePi {
    auth_count: Arc<AtomicUsize>,
    report: String,
    script: Vec<u8>,
}

impl server::Handler for FakePi {
    type Error = russh::Error;
    fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> impl Future<Output = Result<server::Auth, Self::Error>> + Send {
        self.auth_count.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(if user == "gardener" && password == PASSWORD {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        }))
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        assert_eq!(data, b"python3 -I -");
        std::future::ready(session.channel_success(channel))
    }
    fn data(
        &mut self,
        _: ChannelId,
        data: &[u8],
        _: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.script.extend_from_slice(data);
        std::future::ready(Ok(()))
    }
    fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        assert_eq!(self.script, include_bytes!("health.py"));
        let result = session
            .data(channel, self.report.as_bytes().to_vec())
            .and_then(|()| session.exit_status_request(channel, 0))
            .and_then(|()| session.eof(channel))
            .and_then(|()| session.close(channel));
        std::future::ready(result)
    }
}

struct Server {
    port: u16,
    fingerprint: String,
    auth_count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(report: &str) -> Self {
        let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519)
            .expect("test host key");
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("test listener");
        let port = listener.local_addr().expect("address").port();
        let auth_count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&auth_count);
        let report = report.to_owned();
        let task = tokio::runtime::Handle::current().spawn(async move {
            let mut sessions = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.expect("accept");
                        let socket = socket.into_std().expect("standard socket");
                        let guard = SocketGuard(socket.try_clone().expect("socket guard"));
                        let socket = tokio::net::TcpStream::from_std(socket).expect("async socket");
                        let config = Arc::clone(&config);
                        let handler = FakePi { auth_count: Arc::clone(&counter), report: report.clone(), script: Vec::new() };
                        sessions.spawn(async move {
                            let _guard = guard;
                            if let Ok(session) = server::run_stream(config, socket, handler).await { let _ = session.await; }
                        });
                    }
                    Some(result) = sessions.join_next() => { result.expect("server task"); }
                }
            }
        });
        Self {
            port,
            fingerprint,
            auth_count,
            task,
        }
    }
}

fn setup() -> PiSetup {
    PiSetup {
        board_id: "pi-5".into(),
        host: "127.0.0.1".into(),
        user: "gardener".into(),
    }
}
fn store() -> (tempfile::TempDir, Arc<dyn CredentialStore>) {
    let folder = tempfile::tempdir().expect("test credentials");
    let store = Arc::new(sovra_providers::FileCredentialStore::new(
        folder.path().join("secrets"),
    ));
    (folder, store)
}
async fn sign_in(
    server: &Server,
    store: Arc<dyn CredentialStore>,
    password: &str,
    trust: bool,
) -> Result<(), String> {
    let (questions, incoming) = async_channel::bounded::<TrustQuestion>(1);
    let confirm = async {
        let question = incoming.recv().await.expect("identity question");
        assert_eq!(question.fingerprint, server.fingerprint);
        assert_eq!(
            server.auth_count.load(Ordering::SeqCst),
            0,
            "no password before consent"
        );
        question.answer.send(trust).expect("answer");
    };
    let setup = setup();
    tokio::time::timeout(Duration::from_secs(10), async {
        let (result, ()) = tokio::join!(
            pair_at(
                store,
                &setup,
                ModelSecret(password.into()),
                questions,
                server.port
            ),
            confirm
        );
        result
    })
    .await
    .expect("bounded pairing")
}

#[tokio::test]
async fn pairing_trusts_once_saves_after_model_check_and_reconnects() {
    let server = Server::start(REPORT).await;
    let (_folder, store) = store();
    sign_in(&server, Arc::clone(&store), PASSWORD, true)
        .await
        .expect("paired");
    let saved = store.get(&credential_id(&setup())).expect("saved secret");
    assert!(saved.0.contains(&server.fingerprint));
    assert!(!format!("{saved:?}").contains(PASSWORD));
    let output = run_saved_at(
        Some(&store),
        &setup(),
        include_bytes!("health.py"),
        5,
        16384,
        server.port,
    )
    .await
    .expect("reconnect");
    assert_eq!(output.as_deref(), Some(REPORT));
    assert_eq!(server.auth_count.load(Ordering::SeqCst), 2);

    let changed = Server::start(REPORT).await;
    let error = run_saved_at(
        Some(&store),
        &setup(),
        include_bytes!("health.py"),
        5,
        16384,
        changed.port,
    )
    .await
    .expect_err("changed host must stop");
    assert_eq!(error, IDENTITY_CHANGED);
    assert_eq!(changed.auth_count.load(Ordering::SeqCst), 0);
    let error = run_saved_at(
        Some(&store),
        &setup(),
        include_bytes!("health.py"),
        5,
        8,
        server.port,
    )
    .await
    .expect_err("bounded response");
    assert!(error.contains("too much"));
}

#[tokio::test]
async fn rejected_identity_wrong_password_and_wrong_model_never_save_access() {
    for (report, password, trust, expected) in [
        (REPORT, PASSWORD, false, "cancelled"),
        (REPORT, "incorrect", true, "didn't accept"),
        (
            r#"{"model":"Raspberry Pi 4 Model B Rev 1.0"}"#,
            PASSWORD,
            true,
            "differs",
        ),
        ("not json", PASSWORD, true, "couldn't be read"),
    ] {
        let server = Server::start(report).await;
        let (_folder, store) = store();
        let error = sign_in(&server, Arc::clone(&store), password, trust)
            .await
            .expect_err("must fail");
        assert!(error.contains(expected), "{error}");
        assert_eq!(
            store.get(&credential_id(&setup())).expect("credentials").0,
            ""
        );
        if !trust {
            assert_eq!(server.auth_count.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn missing_access_allows_keys_but_corrupt_saved_access_does_not() {
    let (_folder, store) = store();
    assert!(
        run_saved(Some(&store), &setup(), b"", 1, 16)
            .await
            .expect("no saved access")
            .is_none()
    );
    store
        .set(
            &credential_id(&setup()),
            &ModelSecret("broken record".into()),
        )
        .expect("test secret");
    assert_eq!(
        run_saved(Some(&store), &setup(), b"", 1, 16)
            .await
            .expect_err("no fallback"),
        STORE_FAILED
    );
}

#[tokio::test]
async fn cancelling_during_confirmation_stops_the_ssh_handler_without_authentication() {
    let server = Server::start(REPORT).await;
    let (_folder, store) = store();
    let (questions, incoming) = async_channel::bounded::<TrustQuestion>(1);
    let port = server.port;
    let task = tokio::runtime::Handle::current().spawn(async move {
        pair_at(
            store,
            &setup(),
            ModelSecret(PASSWORD.into()),
            questions,
            port,
        )
        .await
    });
    let mut question = tokio::time::timeout(Duration::from_secs(5), incoming.recv())
        .await
        .expect("identity deadline")
        .expect("identity");
    task.abort();
    assert!(task.await.expect_err("cancelled").is_cancelled());
    tokio::time::timeout(Duration::from_secs(2), question.answer.closed())
        .await
        .expect("SSH identity handler stopped");
    assert_eq!(server.auth_count.load(Ordering::SeqCst), 0);
}
