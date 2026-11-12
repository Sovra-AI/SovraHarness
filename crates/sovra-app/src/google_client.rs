//! Shipped Google connect and sign-in, driven from an empty start through the runtime.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sovra_core::google::{ClientSecret, GoogleCommand, GoogleConnector, GoogleEvent, GoogleState};
use sovra_core::{AgentConfig, AuthCommand, AuthEvent, Command, Event};
use sovra_google::{ClientSources, GoogleAdapter, SecretVault};
use sovra_runtime::{Client, RuntimeServices};

const CLIENT_ID: &str = "saved-desktop.apps.googleusercontent.com";
const SECRET: &str = "fixture-desktop-secret-9c2e";
/// The client a local launch supplies through the environment (or the bake),
/// before anyone pastes an id in Settings.
const LAUNCH_CLIENT_ID: &str = "launch-desktop.apps.googleusercontent.com";
const LAUNCH_SECRET: &str = "fixture-launch-secret-4a7b";
/// Website OAuth callbacks. A desktop handoff must not use them.
const WEBSITE_CALLBACKS: [&str; 4] = [
    "localhost:8080",
    "localhost%3A8080",
    "/api/google/callback",
    "run.app",
];
const ACCOUNT_ENTRY: &str = r#"{"refresh_token":"1//refresh-fixture","scope":"openid","account":{"email":"sam@example.com","name":"Sam","granted":[],"turned_off":[]}}"#;

#[derive(Default)]
struct MemoryVault(Mutex<BTreeMap<String, String>>);

impl std::fmt::Debug for MemoryVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryVault")
    }
}

impl SecretVault for MemoryVault {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().expect("test lock").get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0
            .lock()
            .expect("test lock")
            .insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().expect("test lock").remove(key);
        Ok(())
    }
}

struct Session {
    client: Option<Client>,
    events: mpsc::Receiver<Event>,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// The dev file store `cargo xtask dev` actually uses (`SOVRA_DEV=1`).
struct DevCredentialFiles {
    dir: PathBuf,
    vault: Arc<super::CredentialVault>,
}

impl DevCredentialFiles {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sovra-google-dev-credentials-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let vault = Arc::new(super::CredentialVault(Arc::new(
            sovra_providers::FileCredentialStore::new(dir.clone()),
        )));
        Self { dir, vault }
    }
}

impl Drop for DevCredentialFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Session {
    fn start(vault: Arc<dyn SecretVault>) -> Self {
        Self::start_with(vault, ClientSources::default())
    }

    /// `sources` is the same pair `main` passes in: startup environment, else
    /// the client baked from `google-client.env`. The vault starts empty.
    fn start_with(vault: Arc<dyn SecretVault>, sources: ClientSources) -> Self {
        let client = sovra_runtime::start_with_services(
            "test",
            AgentConfig::default(),
            RuntimeServices {
                signin: None,
                google: Some(Arc::new(
                    GoogleAdapter::new(sources, vault).expect("google adapter"),
                )),
                ..RuntimeServices::default()
            },
        )
        .expect("runtime starts");
        let incoming = client.events();
        let (sender, events) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            while let Ok(event) = incoming.recv_blocking() {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            client: Some(client),
            events,
            reader: Some(reader),
        }
    }

    fn send(&self, command: Command) {
        self.client
            .as_ref()
            .expect("runtime is still running")
            .send(command)
            .expect("runtime accepts the command");
    }

    fn wait_for<T>(&self, timeout: Duration, pick: impl Fn(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for the runtime event"
            );
            match self.events.recv_timeout(remaining) {
                Ok(event) => {
                    if let Some(found) = pick(&event) {
                        return found;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for the runtime event");
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("runtime event stream closed");
                }
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(client) = self.client.take() {
            client.shutdown_blocking().expect("runtime stops");
        }
        if let Some(reader) = self.reader.take() {
            reader.join().expect("event reader stops");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            let _ = client.shutdown_blocking();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn assert_google_handoff(url: &str, client_id: &str, secret: &str, sign_in: bool) {
    assert!(
        url.starts_with("https://accounts.google.com/"),
        "authorization URL must be Google's page: {url}"
    );
    assert!(
        url.contains(client_id),
        "authorization URL must name this desktop client: {url}"
    );
    assert!(
        url.contains("127.0.0.1"),
        "authorization URL must use the loopback redirect: {url}"
    );
    assert!(
        !url.contains(secret),
        "authorization URL must not carry the client secret"
    );
    for website in WEBSITE_CALLBACKS {
        assert!(
            !url.contains(website),
            "authorization URL must not use a website callback ({website}): {url}"
        );
    }
    if sign_in {
        assert!(
            url.contains("%2Fcallback"),
            "Continue with Google listens on 127.0.0.1:<port>/callback: {url}"
        );
    } else {
        assert!(
            !url.contains("callback"),
            "Connect Google listens on the 127.0.0.1 port root: {url}"
        );
    }
}

fn both_buttons(session: &Session) {
    both_buttons_for(session, CLIENT_ID, SECRET);
}

fn both_buttons_for(session: &Session, client_id: &str, secret: &str) {
    session.send(Command::Google(GoogleCommand::Connect));
    let connect_url = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::OpenSignIn { url }) => Some(url.clone()),
        _ => None,
    });
    session.send(Command::Auth(AuthCommand::StartGoogleSignIn));
    let sign_in_url = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Auth(AuthEvent::GoogleSignInStarted { auth_url }) => Some(auth_url.clone()),
        _ => None,
    });
    assert_google_handoff(&connect_url, client_id, secret, false);
    assert_google_handoff(&sign_in_url, client_id, secret, true);
    session.send(Command::Google(GoogleCommand::Cancel));
    session.send(Command::Auth(AuthCommand::CancelGoogleSignIn));
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connect_stopped = false;
    let mut sign_in_stopped = false;
    while Instant::now() < deadline && !(connect_stopped && sign_in_stopped) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match session.events.recv_timeout(remaining) {
            Ok(Event::Google(GoogleEvent::Changed(GoogleState::NotConnected))) => {
                connect_stopped = true;
            }
            Ok(Event::Auth(AuthEvent::GoogleSignInCancelled)) => sign_in_stopped = true,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(connect_stopped, "connect hand-off stopped");
    assert!(sign_in_stopped, "sign-in hand-off stopped");
}

#[test]
fn saved_desktop_client_starts_both_buttons_and_survives_restart() {
    let creds = DevCredentialFiles::new();
    let vault = creds.vault.clone();
    let mut session = Session::start(vault.clone());
    let first = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        _ => None,
    });
    assert!(
        matches!(first, GoogleState::Unavailable { .. }),
        "no client anywhere starts on the setup state"
    );
    session.send(Command::Google(GoogleCommand::SaveClient {
        client_id: CLIENT_ID.into(),
        client_secret: Some(ClientSecret::new(SECRET)),
    }));
    let ready = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        _ => None,
    });
    assert_eq!(ready, GoogleState::NotConnected);
    both_buttons(&session);
    session.shutdown();

    let mut restarted = Session::start(vault.clone());
    let loaded = restarted.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        _ => None,
    });
    assert_eq!(loaded, GoogleState::NotConnected);
    both_buttons(&restarted);

    vault
        .set(sovra_google::VAULT_KEY, ACCOUNT_ENTRY)
        .expect("seed account");
    restarted.send(Command::Google(GoogleCommand::Disconnect));
    restarted.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(GoogleState::NotConnected)) => Some(()),
        _ => None,
    });
    assert_eq!(
        vault.get(sovra_google::VAULT_KEY).expect("vault read"),
        None
    );
    let kept = vault
        .get(sovra_google::CLIENT_VAULT_KEY)
        .expect("vault read")
        .expect("client id stays after disconnect");
    assert!(
        kept.contains(CLIENT_ID),
        "disconnect leaves the desktop client id"
    );
    assert!(
        kept.contains(SECRET),
        "the secret stays in the credential store"
    );
    let still = GoogleAdapter::new(ClientSources::default(), vault)
        .expect("google adapter")
        .desktop_client()
        .expect("client remains");
    assert_eq!(still.client_id, CLIENT_ID);
    assert!(
        still.client_secret.as_deref() == Some(SECRET),
        "the saved secret is still the one both buttons use"
    );
    assert!(
        creds.dir.join(sovra_google::CLIENT_VAULT_KEY).is_file(),
        "the client id is one file in the dev credential store"
    );
    assert!(
        !creds.dir.join(sovra_google::VAULT_KEY).exists(),
        "signing out removes only the account file"
    );
    assert!(
        !creds.dir.join("connection").exists(),
        "a dotted credential name is not a directory"
    );
    restarted.shutdown();
}

/// A local launch with the Desktop client in the startup environment (the
/// same slot as a value baked from `google-client.env`) is ready before
/// anyone uses **Use my own Google Cloud client**.
#[test]
fn launch_time_desktop_client_starts_both_buttons_on_loopback_without_save_client() {
    let creds = DevCredentialFiles::new();
    let vault = creds.vault.clone();
    let sources = ClientSources {
        environment: Some(sovra_google::ClientConfig {
            client_id: LAUNCH_CLIENT_ID.into(),
            client_secret: Some(LAUNCH_SECRET.into()),
        }),
        baked: None,
    };
    let mut session = Session::start_with(vault.clone(), sources);
    let first = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        _ => None,
    });
    assert_eq!(
        first,
        GoogleState::NotConnected,
        "a launch-time desktop client starts ready to connect"
    );
    assert_eq!(
        vault
            .get(sovra_google::CLIENT_VAULT_KEY)
            .expect("vault read"),
        None,
        "the empty store has no pasted client"
    );
    both_buttons_for(&session, LAUNCH_CLIENT_ID, LAUNCH_SECRET);
    assert_eq!(
        vault
            .get(sovra_google::CLIENT_VAULT_KEY)
            .expect("vault read"),
        None,
        "starting the buttons does not save a client id"
    );
    session.shutdown();
}

#[test]
fn sign_in_refuses_when_no_desktop_client_exists() {
    let mut session = Session::start(Arc::new(MemoryVault::default()));
    let first = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        _ => None,
    });
    assert!(matches!(first, GoogleState::Unavailable { .. }));
    session.send(Command::Auth(AuthCommand::StartGoogleSignIn));
    let message = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Auth(AuthEvent::Failed { message, .. }) => Some(message.clone()),
        Event::Auth(AuthEvent::GoogleSignInStarted { .. })
        | Event::Google(GoogleEvent::OpenSignIn { .. }) => {
            panic!("sign-in started without a desktop client");
        }
        _ => None,
    });
    assert!(
        message.contains("desktop client id"),
        "sign-in must say a desktop client id is needed"
    );
    assert!(
        !message.contains("accounts.google.com"),
        "sign-in must not open Google without a client"
    );
    assert!(
        !message.contains("this build"),
        "sign-in must not blame the build"
    );
    session.send(Command::Google(GoogleCommand::Connect));
    let again = session.wait_for(Duration::from_secs(20), |event| match event {
        Event::Google(GoogleEvent::Changed(state)) => Some(state.clone()),
        Event::Google(GoogleEvent::OpenSignIn { .. }) => {
            panic!("connect started without a desktop client");
        }
        _ => None,
    });
    assert!(matches!(again, GoogleState::Unavailable { .. }));
    session.shutdown();
}
