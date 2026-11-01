//! Shipped Google connect and sign-in, driven from an empty start through the runtime.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use sovra_core::google::{ClientSecret, GoogleCommand, GoogleConnector, GoogleEvent, GoogleState};
use sovra_core::{AgentConfig, AuthCommand, AuthEvent, Command, Event};
use sovra_google::{ClientSources, GoogleAdapter, SecretVault};
use sovra_runtime::{Client, RuntimeServices};

const CLIENT_ID: &str = "saved-desktop.apps.googleusercontent.com";
const SECRET: &str = "fixture-desktop-secret-9c2e";
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

impl Session {
    fn start(vault: Arc<MemoryVault>) -> Self {
        let client = sovra_runtime::start_with_services(
            "test",
            AgentConfig::default(),
            RuntimeServices {
                signin: None,
                google: Some(Arc::new(
                    GoogleAdapter::new(ClientSources::default(), vault).expect("google adapter"),
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

fn assert_google_handoff(url: &str) {
    assert!(
        url.starts_with("https://accounts.google.com/"),
        "authorization URL must be Google's page"
    );
    assert!(
        url.contains(CLIENT_ID),
        "authorization URL must name this desktop client"
    );
    assert!(
        url.contains("127.0.0.1"),
        "authorization URL must use the loopback redirect"
    );
    assert!(
        !url.contains(SECRET),
        "authorization URL must not carry the client secret"
    );
    assert!(
        !url.contains("isn't set up"),
        "a saved client must start the browser hand-off"
    );
}

fn both_buttons(session: &Session) {
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
    assert_google_handoff(&connect_url);
    assert_google_handoff(&sign_in_url);
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
    let vault = Arc::new(MemoryVault::default());
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
    restarted.shutdown();
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
