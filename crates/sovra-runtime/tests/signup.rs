//! Signup through the shipped create and sign-in commands, from an empty store.
//!
//! The assertions read what `AuthCommand::CreateAccount` and `AuthCommand::SignIn`
//! persist. They do not call the signup helper themselves.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use sovra_core::auth::{AuthAccount, AuthCommand, AuthEvent, AuthField, AuthRecords};
use sovra_core::{
    AgentConfig, Command, Episode, EpisodeId, Event, FactId, MemoryCandidate, MemoryClaim,
    MemoryError, MemoryExtractedChunk, MemoryInput, MemoryJob, MemoryPolicy, MemoryStore,
    NewEpisode, NewFact, RecallQuery, RecalledFact, Timestamp,
};
use sovra_runtime::{Client, RuntimeServices};

const WAIT: Duration = Duration::from_secs(60);

struct AuthMemory(Mutex<Option<AuthRecords>>);

impl AuthMemory {
    fn accounts(&self) -> Vec<AuthAccount> {
        self.0
            .lock()
            .expect("auth store lock")
            .as_ref()
            .map(|records| records.accounts.clone())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl MemoryStore for AuthMemory {
    async fn health(&self) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn memory_policy(&self) -> Result<MemoryPolicy, MemoryError> {
        Ok(MemoryPolicy {
            enabled: false,
            generation: 0,
        })
    }

    async fn set_memory_enabled(&self, _: bool) -> Result<MemoryPolicy, MemoryError> {
        self.memory_policy().await
    }

    async fn enqueue_memory(&self, _: MemoryInput) -> Result<EpisodeId, MemoryError> {
        Err(MemoryError::Backend("unused in the signup test".into()))
    }

    async fn claim_memory(&self, _: Timestamp) -> Result<Option<MemoryClaim>, MemoryError> {
        Ok(None)
    }

    async fn save_memory_extraction(
        &self,
        _: &MemoryClaim,
        _: &MemoryExtractedChunk,
        _: Timestamp,
    ) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn complete_memory(
        &self,
        _: &MemoryClaim,
        _: Vec<MemoryCandidate>,
        _: &str,
        _: Timestamp,
    ) -> Result<Vec<FactId>, MemoryError> {
        Ok(Vec::new())
    }

    async fn fail_memory(&self, _: &MemoryClaim, _: &str, _: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn memory_job(&self, id: &EpisodeId) -> Result<MemoryJob, MemoryError> {
        Err(MemoryError::Backend(format!("no job for {}", id.0)))
    }

    async fn record_episode(&self, _: NewEpisode) -> Result<EpisodeId, MemoryError> {
        Err(MemoryError::Backend("unused in the signup test".into()))
    }

    async fn episode(&self, _: &EpisodeId) -> Result<Episode, MemoryError> {
        Err(MemoryError::Backend("unused in the signup test".into()))
    }

    async fn recent_episodes(&self, _: &str, _: usize) -> Result<Vec<Episode>, MemoryError> {
        Ok(Vec::new())
    }

    async fn save_fact(&self, _: NewFact) -> Result<FactId, MemoryError> {
        Err(MemoryError::Backend("unused in the signup test".into()))
    }

    async fn invalidate_fact(&self, _: &FactId, _: Timestamp) -> Result<(), MemoryError> {
        Ok(())
    }

    async fn recall(&self, _: RecallQuery) -> Result<Vec<RecalledFact>, MemoryError> {
        Ok(Vec::new())
    }

    async fn load_auth(&self) -> Result<Option<AuthRecords>, MemoryError> {
        Ok(self.0.lock().expect("auth store lock").clone())
    }

    async fn save_auth(&self, records: &AuthRecords) -> Result<(), MemoryError> {
        *self.0.lock().expect("auth store lock") = Some(records.clone());
        Ok(())
    }
}

struct Session {
    client: Option<Client>,
    events: mpsc::Receiver<Event>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    fn start(store: Arc<AuthMemory>) -> Self {
        let memory: sovra_runtime::MemoryFactory =
            Box::new(move || Box::pin(async move { Ok(store as Arc<dyn MemoryStore>) }));
        let client = sovra_runtime::start_with_services(
            "signup-test",
            AgentConfig::default(),
            RuntimeServices {
                memory: Some(memory),
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

    fn wait_for<T>(&self, pick: impl Fn(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for the signup event"
            );
            match self.events.recv_timeout(remaining) {
                Ok(event) => {
                    if let Some(found) = pick(&event) {
                        return found;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("timed out waiting for the signup event");
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

fn create(session: &Session, email: &str, password: &str, full_name: &str) {
    session.send(Command::Auth(AuthCommand::CreateAccount {
        email: email.into(),
        password: password.into(),
        full_name: full_name.into(),
    }));
}

#[test]
fn signup_stores_the_typed_name_and_keeps_email_password_optional() {
    let store = Arc::new(AuthMemory(Mutex::new(None)));
    let mut session = Session::start(store.clone());

    create(&session, "sam@example.com", "longenough", "   ");
    let (field, message) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::Failed { field, message }) => Some((*field, message.clone())),
        _ => None,
    });
    assert_eq!(field, AuthField::FullName);
    assert_eq!(message, "Enter your name.");
    assert_eq!(
        store.accounts(),
        Vec::<AuthAccount>::new(),
        "a blank name was stored"
    );

    create(&session, "sam@example.com", "", "Sam Rivera");
    let (field, _) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::Failed { field, message }) => Some((*field, message.clone())),
        _ => None,
    });
    assert_eq!(field, AuthField::Password);
    assert_eq!(store.accounts(), Vec::<AuthAccount>::new());

    create(&session, "", "longenough", "Sam Rivera");
    let (field, _) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::Failed { field, message }) => Some((*field, message.clone())),
        _ => None,
    });
    assert_eq!(field, AuthField::Email);
    assert!(
        store
            .accounts()
            .iter()
            .all(|account| account.full_name != "sam"),
        "email local-part was stored: {:?}",
        store.accounts()
    );

    create(&session, "sam@example.com", "short", "Sam Rivera");
    let (field, message) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::Failed { field, message }) => Some((*field, message.clone())),
        _ => None,
    });
    assert_eq!(field, AuthField::Password);
    assert!(message.contains('8'), "{message}");
    assert_eq!(store.accounts(), Vec::<AuthAccount>::new());

    create(&session, "", "", "Sam Rivera");
    let (session_name, new_account) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::SignedIn {
            session,
            new_account,
        }) => Some((session.full_name.clone(), *new_account)),
        _ => None,
    });
    assert!(new_account);
    assert_eq!(session_name, "Sam Rivera");
    let name_only = store
        .accounts()
        .into_iter()
        .find(|account| account.full_name == "Sam Rivera")
        .expect("name-only account");
    assert_eq!(name_only.email, "");
    assert_eq!(name_only.password_phc, None);
    assert_ne!(name_only.full_name, "sam");

    create(&session, "sam@example.com", "longenough", "Sam Rivera");
    let (email, name, new_account) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::SignedIn {
            session,
            new_account,
        }) => Some((
            session.email.clone(),
            session.full_name.clone(),
            *new_account,
        )),
        _ => None,
    });
    assert!(new_account);
    assert_eq!(email, "sam@example.com");
    assert_eq!(name, "Sam Rivera");
    let password_account = store
        .accounts()
        .into_iter()
        .find(|account| account.email == "sam@example.com")
        .expect("email account");
    assert_eq!(password_account.full_name, "Sam Rivera");
    let phc = password_account.password_phc.expect("password hash");
    assert!(
        phc.starts_with("$argon2"),
        "stored password is not a hash: {phc}"
    );

    session.send(Command::Auth(AuthCommand::SignOut));
    session.wait_for(|event| match event {
        Event::Auth(AuthEvent::SignedOut) => Some(()),
        _ => None,
    });
    session.send(Command::Auth(AuthCommand::SignIn {
        email: "sam@example.com".into(),
        password: "longenough".into(),
    }));
    let (name, new_account) = session.wait_for(|event| match event {
        Event::Auth(AuthEvent::SignedIn {
            session,
            new_account,
        }) => Some((session.full_name.clone(), *new_account)),
        _ => None,
    });
    assert!(!new_account);
    assert_eq!(name, "Sam Rivera");

    session.shutdown();
}
