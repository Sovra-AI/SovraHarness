//! Microsoft connection lifecycle through the real runtime's command/event channels.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sovra_core::microsoft::{
    MicrosoftAccount, MicrosoftClient, MicrosoftCommand, MicrosoftConnector, MicrosoftError,
    MicrosoftEvent, MicrosoftOutput, MicrosoftRequest, MicrosoftService, MicrosoftSession,
    MicrosoftState,
};
use sovra_core::oauth::{AuthorizationGrant, NativeSignIn, OAuthError, SignInCanceller};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, RuntimeServices};

#[derive(Debug)]
struct Script {
    saved: Option<MicrosoftAccount>,
    client: Option<MicrosoftClient>,
    requested: Vec<BTreeSet<MicrosoftService>>,
    next_id: Option<String>,
    wait_for_cancel: bool,
    fail_save: bool,
    cancelled: bool,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            saved: None,
            client: Some(config()),
            requested: Vec::new(),
            next_id: None,
            wait_for_cancel: false,
            fail_save: false,
            cancelled: false,
        }
    }
}

#[derive(Debug, Default)]
struct FakeMicrosoft(Arc<Mutex<Script>>);

#[derive(Debug)]
struct SignIn {
    ready: std::sync::mpsc::Receiver<()>,
    wake: std::sync::mpsc::Sender<()>,
    script: Arc<Mutex<Script>>,
    waits: bool,
}

impl NativeSignIn for SignIn {
    fn url(&self) -> &'static str {
        "https://login.microsoftonline.com/common/oauth2/v2.0/authorize?fixture"
    }
    fn canceller(&self) -> SignInCanceller {
        let wake = self.wake.clone();
        let script = self.script.clone();
        Arc::new(move || {
            script.lock().expect("script").cancelled = true;
            let _ = wake.send(());
        })
    }
    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, OAuthError> {
        if self.waits {
            let _ = self.ready.recv_timeout(Duration::from_secs(5));
            return Err(OAuthError::Cancelled);
        }
        Ok(AuthorizationGrant {
            code: "fixture".into(),
            verifier: "fixture".into(),
            redirect_uri: "http://localhost:1".into(),
        })
    }
}

#[derive(Debug)]
struct Session {
    account: MicrosoftAccount,
    script: Arc<Mutex<Script>>,
}

#[async_trait::async_trait]
impl MicrosoftSession for Session {
    fn account(&self) -> MicrosoftAccount {
        self.account.clone()
    }
    async fn save(&self, account: &MicrosoftAccount) -> Result<(), MicrosoftError> {
        let mut script = self.script.lock().expect("script");
        if script.fail_save {
            return Err(MicrosoftError::Failed(
                "Credential store unavailable.".into(),
            ));
        }
        script.saved = Some(account.clone());
        Ok(())
    }
    async fn disconnect(&self) -> Result<(), MicrosoftError> {
        self.script.lock().expect("script").saved = None;
        Ok(())
    }
    async fn retire(&self) {}
    async fn call(&self, _: MicrosoftRequest) -> Result<MicrosoftOutput, MicrosoftError> {
        Ok(MicrosoftOutput::Data(serde_json::Value::Null))
    }
}

#[async_trait::async_trait]
impl MicrosoftConnector for FakeMicrosoft {
    async fn load_client(&self) -> Result<Option<MicrosoftClient>, MicrosoftError> {
        let script = self.0.lock().expect("script");
        Ok(script.client.clone())
    }
    async fn save_client(&self, client: &MicrosoftClient) -> Result<(), MicrosoftError> {
        let mut script = self.0.lock().expect("script");
        if script.fail_save {
            return Err(MicrosoftError::Failed(
                "Credential store unavailable.".into(),
            ));
        }
        script.client = Some(client.clone());
        Ok(())
    }
    async fn forget_client(&self) -> Result<(), MicrosoftError> {
        let mut script = self.0.lock().expect("script");
        script.client = None;
        script.saved = None;
        Ok(())
    }
    fn begin_sign_in(
        &self,
        _: &MicrosoftClient,
        services: &BTreeSet<MicrosoftService>,
        _: Option<&str>,
    ) -> Result<Box<dyn NativeSignIn>, MicrosoftError> {
        let mut script = self.0.lock().expect("script");
        script.requested.push(services.clone());
        let (wake, ready) = std::sync::mpsc::channel();
        Ok(Box::new(SignIn {
            ready,
            wake,
            script: self.0.clone(),
            waits: script.wait_for_cancel,
        }))
    }
    async fn restore(
        &self,
        _: &MicrosoftClient,
    ) -> Result<Option<Arc<dyn MicrosoftSession>>, MicrosoftError> {
        Ok(self.0.lock().expect("script").saved.clone().map(|account| {
            Arc::new(Session {
                account,
                script: self.0.clone(),
            }) as Arc<dyn MicrosoftSession>
        }))
    }
    async fn complete_sign_in(
        &self,
        _: &MicrosoftClient,
        _: AuthorizationGrant,
    ) -> Result<Arc<dyn MicrosoftSession>, MicrosoftError> {
        let script = self.0.lock().expect("script");
        let mut granted = script.requested.last().cloned().unwrap_or_default();
        // Tasks.ReadWrite also covers Planner. It must remain off until the user chooses it.
        granted.insert(MicrosoftService::Planner);
        let account = MicrosoftAccount {
            id: script.next_id.clone().unwrap_or_else(|| "sam".into()),
            name: "Sam".into(),
            email: "sam@example.com".into(),
            granted,
            turned_off: BTreeSet::new(),
        };
        Ok(Arc::new(Session {
            account,
            script: self.0.clone(),
        }))
    }
}

fn start(adapter: Arc<FakeMicrosoft>) -> Client {
    sovra_runtime::start_with_services(
        "test",
        AgentConfig::default(),
        RuntimeServices {
            microsoft: Some(adapter),
            ..RuntimeServices::default()
        },
    )
    .expect("runtime")
}

fn send(client: &Client, command: MicrosoftCommand) {
    client.send(Command::Microsoft(command)).expect("command");
}

fn until(client: &Client, expected: impl Fn(&MicrosoftEvent) -> bool) -> MicrosoftEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match client.events().try_recv() {
            Ok(Event::Microsoft(event)) if expected(&event) => return event,
            Ok(_) | Err(async_channel::TryRecvError::Empty) => std::thread::yield_now(),
            Err(async_channel::TryRecvError::Closed) => {
                panic!("runtime stopped before expected event")
            }
        }
    }
    panic!("timed out waiting for Microsoft event")
}

fn connected(client: &Client) -> MicrosoftAccount {
    match until(client, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::Connected(_)))
    }) {
        MicrosoftEvent::Changed(MicrosoftState::Connected(account)) => account,
        _ => unreachable!("predicate checked"),
    }
}

#[test]
fn one_connect_restores_switches_adds_optional_access_and_disconnects() {
    let adapter = Arc::new(FakeMicrosoft::default());
    let client = start(adapter.clone());
    until(&client, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    send(&client, MicrosoftCommand::Connect);
    let account = connected(&client);
    assert!(account.active(MicrosoftService::Mail));
    assert!(!account.active(MicrosoftService::Planner));
    assert_eq!(
        adapter.0.lock().expect("script").requested[0],
        MicrosoftService::defaults()
    );
    send(
        &client,
        MicrosoftCommand::SetService {
            service: MicrosoftService::Mail,
            on: false,
        },
    );
    assert!(!connected(&client).active(MicrosoftService::Mail));
    send(
        &client,
        MicrosoftCommand::SetService {
            service: MicrosoftService::Channels,
            on: true,
        },
    );
    let account = connected(&client);
    assert!(account.active(MicrosoftService::Channels));
    assert!(!account.active(MicrosoftService::Mail));
    client.send(Command::Shutdown).expect("shutdown");
    let next = start(adapter.clone());
    let account = connected(&next);
    assert!(account.active(MicrosoftService::Channels));
    assert!(!account.active(MicrosoftService::Mail));
    send(&next, MicrosoftCommand::Disconnect);
    until(&next, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    next.send(Command::Ping)
        .expect("ping after disconnect completes");
    let deadline = Instant::now() + Duration::from_secs(5);
    while adapter.0.lock().expect("script").saved.is_some() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(adapter.0.lock().expect("script").saved.is_none());
    next.send(Command::Shutdown).expect("shutdown");
}

#[test]
fn cancellation_wakes_the_browser_wait_and_cannot_save_a_late_result() {
    let adapter = Arc::new(FakeMicrosoft::default());
    adapter.0.lock().expect("script").wait_for_cancel = true;
    let client = start(adapter.clone());
    until(&client, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    send(&client, MicrosoftCommand::Connect);
    until(&client, |event| {
        matches!(event, MicrosoftEvent::OpenSignIn { .. })
    });
    send(&client, MicrosoftCommand::Cancel);
    until(&client, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !adapter.0.lock().expect("script").cancelled && Instant::now() < deadline {
        std::thread::yield_now();
    }
    let script = adapter.0.lock().expect("script");
    assert!(script.cancelled);
    assert!(script.saved.is_none());
    drop(script);
    client.send(Command::Shutdown).expect("shutdown");
}

#[test]
fn sign_in_is_not_connected_until_saved_and_cannot_replace_a_different_account() {
    let adapter = Arc::new(FakeMicrosoft::default());
    adapter.0.lock().expect("script").fail_save = true;
    let client = start(adapter.clone());
    until(&client, |event| {
        matches!(event, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    send(&client, MicrosoftCommand::Connect);
    until(&client, |event| matches!(event, MicrosoftEvent::Notice(_)));
    assert!(adapter.0.lock().expect("script").saved.is_none());
    adapter.0.lock().expect("script").fail_save = false;
    send(&client, MicrosoftCommand::Connect);
    assert_eq!(connected(&client).id, "sam");
    adapter.0.lock().expect("script").next_id = Some("different".into());
    send(
        &client,
        MicrosoftCommand::SetService {
            service: MicrosoftService::Channels,
            on: true,
        },
    );
    until(
        &client,
        |event| matches!(event, MicrosoftEvent::Notice(message) if message.contains("same Microsoft account")),
    );
    assert_eq!(
        adapter
            .0
            .lock()
            .expect("script")
            .saved
            .as_ref()
            .expect("saved account")
            .id,
        "sam"
    );
    client.send(Command::Shutdown).expect("shutdown");
}

fn config() -> MicrosoftClient {
    MicrosoftClient {
        client_id: "10000000-0000-4000-8000-000000000001".into(),
        tenant: "consumers".into(),
    }
}

#[test]
fn personal_setup_saves_connects_restores_and_forgets_without_a_publisher_default() {
    let adapter = Arc::new(FakeMicrosoft::default());
    adapter.0.lock().expect("script").client = None;
    let client = start(adapter.clone());
    until(&client, |e| {
        matches!(e, MicrosoftEvent::Changed(MicrosoftState::NeedsSetup))
    });
    send(&client, MicrosoftCommand::Connect);
    until(&client, |e| {
        matches!(e, MicrosoftEvent::Changed(MicrosoftState::NeedsSetup))
    });
    assert_eq!(adapter.0.lock().expect("script").requested.len(), 0);
    let mut invalid = config();
    invalid.client_id = "an-email@example.com".into();
    send(&client, MicrosoftCommand::SaveAndConnect(invalid));
    until(&client, |e| matches!(e, MicrosoftEvent::Notice(_)));
    assert!(adapter.0.lock().expect("script").client.is_none());
    send(&client, MicrosoftCommand::SaveAndConnect(config()));
    until(
        &client,
        |e| matches!(e, MicrosoftEvent::SetupChanged(Some(c)) if c == &config()),
    );
    connected(&client);
    send(&client, MicrosoftCommand::ForgetSetup);
    until(&client, |e| matches!(e, MicrosoftEvent::Notice(_)));
    assert_eq!(adapter.0.lock().expect("script").client, Some(config()));
    client.send(Command::Shutdown).expect("shutdown");
    let next = start(adapter.clone());
    connected(&next);
    send(&next, MicrosoftCommand::Disconnect);
    until(&next, |e| {
        matches!(e, MicrosoftEvent::Changed(MicrosoftState::NotConnected))
    });
    send(&next, MicrosoftCommand::ForgetSetup);
    until(&next, |e| matches!(e, MicrosoftEvent::SetupChanged(None)));
    until(&next, |e| {
        matches!(e, MicrosoftEvent::Changed(MicrosoftState::NeedsSetup))
    });
    assert!(adapter.0.lock().expect("script").client.is_none());
    assert!(adapter.0.lock().expect("script").saved.is_none());
    next.send(Command::Shutdown).expect("shutdown");
}

#[test]
fn failing_to_save_setup_does_not_open_a_browser_or_claim_it_is_saved() {
    let adapter = Arc::new(FakeMicrosoft::default());
    {
        let mut script = adapter.0.lock().expect("script");
        script.client = None;
        script.fail_save = true;
    }
    let client = start(adapter.clone());
    until(&client, |e| {
        matches!(e, MicrosoftEvent::Changed(MicrosoftState::NeedsSetup))
    });
    send(&client, MicrosoftCommand::SaveAndConnect(config()));
    until(&client, |e| matches!(e, MicrosoftEvent::Notice(_)));
    let script = adapter.0.lock().expect("script");
    assert!(script.client.is_none());
    assert_eq!(script.requested.len(), 0);
    client.send(Command::Shutdown).expect("shutdown");
}
