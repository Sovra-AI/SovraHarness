//! The Google connection through the real runtime, with a scripted connector.

#![allow(
    clippy::unwrap_used,
    reason = "integration tests fail loudly by design"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use sovra_core::google::{
    AuthorizationGrant, GoogleAccount, GoogleCommand, GoogleConnector, GoogleError, GoogleEvent,
    GoogleRequest, GoogleService, GoogleSignIn, GoogleState, SignInCanceller,
};
use sovra_core::{AgentConfig, Command, Event};
use sovra_runtime::{Client, RuntimeServices};

#[derive(Debug, Default)]
struct Script {
    saved: Option<GoogleAccount>,
    /// What the next sign-in grants.
    grants: BTreeSet<GoogleService>,
    asked_for: Vec<BTreeSet<GoogleService>>,
    revoked: bool,
}

#[derive(Debug, Default)]
struct FakeGoogle(Mutex<Script>);

#[derive(Debug)]
struct InstantSignIn;

impl GoogleSignIn for InstantSignIn {
    fn url(&self) -> &'static str {
        "https://accounts.google.com/o/oauth2/v2/auth?test"
    }
    fn canceller(&self) -> SignInCanceller {
        Arc::new(|| {})
    }
    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, GoogleError> {
        Ok(AuthorizationGrant {
            code: "c".into(),
            verifier: "v".into(),
            redirect_uri: "http://127.0.0.1:1".into(),
        })
    }
}

#[async_trait::async_trait]
impl GoogleConnector for FakeGoogle {
    fn unavailable_reason(&self) -> Option<String> {
        None
    }
    fn desktop_client(&self) -> Option<sovra_core::google::DesktopClient> {
        None
    }
    fn save_desktop_client(&self, _: &str, _: Option<String>) -> Result<(), GoogleError> {
        Ok(())
    }
    fn restore(&self) -> Result<Option<GoogleAccount>, GoogleError> {
        Ok(self.0.lock().unwrap().saved.clone())
    }
    fn begin_sign_in(
        &self,
        services: &BTreeSet<GoogleService>,
        _: Option<&str>,
    ) -> Result<Box<dyn GoogleSignIn>, GoogleError> {
        self.0.lock().unwrap().asked_for.push(services.clone());
        Ok(Box::new(InstantSignIn))
    }
    async fn complete_sign_in(&self, _: AuthorizationGrant) -> Result<GoogleAccount, GoogleError> {
        Ok(GoogleAccount {
            email: "sam@example.com".into(),
            name: "Sam".into(),
            granted: self.0.lock().unwrap().grants.clone(),
            turned_off: BTreeSet::new(),
        })
    }
    fn save(&self, account: &GoogleAccount) -> Result<(), GoogleError> {
        self.0.lock().unwrap().saved = Some(account.clone());
        Ok(())
    }
    async fn revoke(&self) -> Result<(), GoogleError> {
        self.0.lock().unwrap().revoked = true;
        Ok(())
    }
    fn forget(&self) -> Result<(), GoogleError> {
        self.0.lock().unwrap().saved = None;
        Ok(())
    }
    async fn call(&self, _: GoogleRequest) -> Result<serde_json::Value, GoogleError> {
        Ok(serde_json::Value::Null)
    }
}

fn start(google: Arc<FakeGoogle>) -> Client {
    sovra_runtime::start_with_services(
        "test",
        AgentConfig::default(),
        RuntimeServices {
            google: Some(google),
            ..RuntimeServices::default()
        },
    )
    .unwrap()
}

/// The next Google state change, skipping other events.
fn next_state(client: &Client) -> GoogleState {
    loop {
        if let Event::Google(GoogleEvent::Changed(state)) = client.events().recv_blocking().unwrap()
        {
            return state;
        }
    }
}

fn send(client: &Client, command: GoogleCommand) {
    client.send(Command::Google(command)).unwrap();
}

#[test]
fn connect_switch_off_turn_on_and_disconnect() {
    let google = Arc::new(FakeGoogle::default());
    google.0.lock().unwrap().grants = BTreeSet::from([GoogleService::Mail, GoogleService::Tasks]);
    let client = start(google.clone());
    assert_eq!(
        next_state(&client),
        GoogleState::NotConnected,
        "loads on start"
    );

    send(&client, GoogleCommand::Connect);
    assert!(matches!(
        next_state(&client),
        GoogleState::Connecting { adding: None, .. }
    ));
    let GoogleState::Connected(account) = next_state(&client) else {
        panic!("expected connected");
    };
    assert_eq!(
        account.active(),
        BTreeSet::from([GoogleService::Mail, GoogleService::Tasks])
    );
    assert_eq!(
        google.0.lock().unwrap().asked_for[0],
        GoogleService::ALL.into_iter().collect(),
        "the one button asks for everything"
    );

    send(
        &client,
        GoogleCommand::SetService {
            service: GoogleService::Mail,
            on: false,
        },
    );
    let GoogleState::Connected(account) = next_state(&client) else {
        panic!("expected connected");
    };
    assert!(!account.is_active(GoogleService::Mail));
    assert_eq!(
        google.0.lock().unwrap().saved.as_ref(),
        Some(&account),
        "kept in the keychain"
    );

    // Calendar was never allowed: turning it on asks Google again, keeping the rest.
    google.0.lock().unwrap().grants = BTreeSet::from([
        GoogleService::Mail,
        GoogleService::Tasks,
        GoogleService::Calendar,
    ]);
    send(
        &client,
        GoogleCommand::SetService {
            service: GoogleService::Calendar,
            on: true,
        },
    );
    assert!(matches!(
        next_state(&client),
        GoogleState::Connecting {
            adding: Some(GoogleService::Calendar),
            ..
        }
    ));
    let GoogleState::Connected(account) = next_state(&client) else {
        panic!("expected connected");
    };
    assert_eq!(
        google.0.lock().unwrap().asked_for[1],
        BTreeSet::from([
            GoogleService::Mail,
            GoogleService::Tasks,
            GoogleService::Calendar
        ])
    );
    assert!(account.is_active(GoogleService::Calendar));
    assert!(
        !account.is_active(GoogleService::Mail),
        "still switched off"
    );

    send(&client, GoogleCommand::Disconnect);
    assert_eq!(next_state(&client), GoogleState::NotConnected);
    let script = google.0.lock().unwrap();
    assert!(script.revoked && script.saved.is_none());
}

#[test]
fn a_saved_account_is_restored_on_start() {
    let google = Arc::new(FakeGoogle::default());
    let saved = GoogleAccount {
        email: "sam@example.com".into(),
        granted: BTreeSet::from([GoogleService::Files]),
        ..GoogleAccount::default()
    };
    google.0.lock().unwrap().saved = Some(saved.clone());
    let client = start(google);
    assert_eq!(next_state(&client), GoogleState::Connected(saved));
}
