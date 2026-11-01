//! Google: one "Connect Google" sign-in and the Google services Sovra may use with it.
//!
//! The person presses one button, signs in on Google's own page in their browser
//! and comes back connected. Each service is one row in [`SERVICES`] (its plain
//! words, icon and OAuth scopes), so the scopes Sovra asks for, what the page
//! shows and which agent tools exist all come from that one table. A service
//! the account allowed can be switched off here without signing out; turning on
//! one Google was not allowed to use asks Google for just that service.
//!
//! Sign-in, the keychain and HTTP live behind [`GoogleConnector`], implemented by
//! `sovra-google`. Agent tools only ever see [`GoogleRequest`]s.

mod port;
mod request;
mod text;

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

pub use port::{AuthorizationGrant, GoogleConnector, GoogleSignIn, SignInCanceller};
pub use request::{GoogleRequest, MAX_RESULTS, result_limit};
pub use text::{clip_text, html_to_text};

/// What a connected Google account lets Sovra do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleService {
    Mail,
    Calendar,
    Files,
    Contacts,
    Tasks,
}

/// Facts about one service. `icon` is a semantic key the UI maps to its icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceInfo {
    pub service: GoogleService,
    /// What people call it ("Email"), not the product name.
    pub label: &'static str,
    pub product: &'static str,
    /// One plain sentence: what Sovra does with it, including what it may change.
    pub explanation: &'static str,
    pub icon: &'static str,
    /// Least privilege for the tools in [`GoogleRequest`]; all must be granted.
    pub scopes: &'static [&'static str],
}

/// Asked with every sign-in so Sovra can show who is connected.
pub const GOOGLE_IDENTITY_SCOPES: [&str; 3] = ["openid", "email", "profile"];

const MAIL_READ: &str = "https://www.googleapis.com/auth/gmail.readonly";
const MAIL_DRAFT: &str = "https://www.googleapis.com/auth/gmail.compose";

/// One row per service, in the order the Connections page lists them.
pub const SERVICES: [ServiceInfo; 5] = [
    ServiceInfo {
        service: GoogleService::Mail,
        label: "Email",
        product: "Gmail",
        explanation: "Find and read your emails, and write drafts for you to check and send \
                      yourself. Sovra never sends email on its own.",
        icon: "mail",
        scopes: &[MAIL_READ, MAIL_DRAFT],
    },
    ServiceInfo {
        service: GoogleService::Calendar,
        label: "Calendar",
        product: "Google Calendar",
        explanation: "See what's coming up and add events when you ask.",
        icon: "calendar",
        scopes: &["https://www.googleapis.com/auth/calendar.events"],
    },
    ServiceInfo {
        service: GoogleService::Files,
        label: "Files",
        product: "Google Drive",
        explanation: "Find and read your documents, spreadsheets and slides. Sovra only reads them.",
        icon: "files",
        scopes: &["https://www.googleapis.com/auth/drive.readonly"],
    },
    ServiceInfo {
        service: GoogleService::Contacts,
        label: "Contacts",
        product: "Google Contacts",
        explanation: "Look up people's email addresses and phone numbers. Sovra only reads them.",
        icon: "contacts",
        scopes: &["https://www.googleapis.com/auth/contacts.readonly"],
    },
    ServiceInfo {
        service: GoogleService::Tasks,
        label: "To-dos",
        product: "Google Tasks",
        explanation: "See your to-do list and add things to it when you ask.",
        icon: "tasks",
        scopes: &["https://www.googleapis.com/auth/tasks"],
    },
];

pub fn service_info(service: GoogleService) -> &'static ServiceInfo {
    let index = match service {
        GoogleService::Mail => 0,
        GoogleService::Calendar => 1,
        GoogleService::Files => 2,
        GoogleService::Contacts => 3,
        GoogleService::Tasks => 4,
    };
    &SERVICES[index]
}

impl GoogleService {
    pub const ALL: [Self; 5] = [
        Self::Mail,
        Self::Calendar,
        Self::Files,
        Self::Contacts,
        Self::Tasks,
    ];
}

/// The scopes to request for `services`: identity first, no duplicates.
pub fn scopes_for(services: &[GoogleService]) -> Vec<&'static str> {
    let mut scopes: Vec<&'static str> = GOOGLE_IDENTITY_SCOPES.to_vec();
    for service in services {
        for scope in service_info(*service).scopes {
            if !scopes.contains(scope) {
                scopes.push(scope);
            }
        }
    }
    scopes
}

/// The services a token response's space-separated `scope` actually covers.
///
/// Google's consent screen lets people untick individual permissions, so a
/// service counts only when every scope it needs came back.
pub fn granted_services(granted_scope: &str) -> BTreeSet<GoogleService> {
    let granted: BTreeSet<&str> = granted_scope.split_whitespace().collect();
    GoogleService::ALL
        .into_iter()
        .filter(|service| {
            service_info(*service)
                .scopes
                .iter()
                .all(|scope| granted.contains(scope))
        })
        .collect()
}

/// The signed-in account. Tokens never leave `sovra-google`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GoogleAccount {
    pub email: String,
    pub name: String,
    /// What Google allowed at the last sign-in.
    pub granted: BTreeSet<GoogleService>,
    /// Allowed, but switched off by the person on the Connections page.
    pub turned_off: BTreeSet<GoogleService>,
}

impl GoogleAccount {
    pub fn is_active(&self, service: GoogleService) -> bool {
        self.granted.contains(&service) && !self.turned_off.contains(&service)
    }

    pub fn active(&self) -> BTreeSet<GoogleService> {
        self.granted.difference(&self.turned_off).copied().collect()
    }

    /// A first letter for the avatar ("S" for sam@example.com).
    pub fn initial(&self) -> String {
        self.name
            .chars()
            .chain(self.email.chars())
            .find(|c| c.is_alphanumeric())
            .map_or_else(|| "G".into(), |c| c.to_uppercase().collect())
    }
}

/// What Settings says when this computer has no desktop client id yet.
pub const DESKTOP_CLIENT_NEEDED: &str = "Google needs a desktop client id from your Google Cloud \
     project before it can connect. Paste the id below. A secret is optional, and Sovra does not \
     show it again after you save.";

/// A Google Cloud desktop OAuth client. The secret is optional (PKCE) and never
/// appears in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct DesktopClient {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for DesktopClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopClient")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Trims the id and secret. A blank id is absent. A blank secret is dropped.
pub fn normalize_desktop_client(mut client: DesktopClient) -> Option<DesktopClient> {
    let client_id = client.client_id.trim().to_owned();
    if client_id.is_empty() {
        return None;
    }
    client.client_id = client_id;
    client.client_secret = client.client_secret.and_then(|secret| {
        let secret = secret.trim().to_owned();
        (!secret.is_empty()).then_some(secret)
    });
    Some(client)
}

/// Which desktop client is in force: one saved on this computer, else the
/// environment, else one baked into the binary. A blank id counts as absent.
pub fn client_in_force(
    saved: Option<DesktopClient>,
    environment: Option<DesktopClient>,
    baked: Option<DesktopClient>,
) -> Option<DesktopClient> {
    [saved, environment, baked]
        .into_iter()
        .find_map(|client| client.and_then(normalize_desktop_client))
}

/// A desktop-client secret carried in a command. `Debug` never includes it.
#[derive(Clone, PartialEq, Eq)]
pub struct ClientSecret(String);

impl ClientSecret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSecret([redacted])")
    }
}

/// Where the Google connection is. The runtime always sends the whole state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GoogleState {
    /// Not yet known (before the runtime has looked in the keychain).
    #[default]
    Loading,
    /// No desktop client id is saved, set in the environment, or baked in.
    Unavailable {
        reason: String,
    },
    NotConnected,
    /// Waiting for the person to finish on Google's page in their browser.
    Connecting {
        url: String,
        /// Set when turning on one more service for an existing account.
        adding: Option<GoogleService>,
    },
    Connected(GoogleAccount),
    /// Google stopped accepting the saved sign-in; signing in again fixes it.
    NeedsSignIn(GoogleAccount),
}

impl GoogleState {
    pub fn account(&self) -> Option<&GoogleAccount> {
        match self {
            Self::Connected(account) | Self::NeedsSignIn(account) => Some(account),
            Self::Loading
            | Self::Unavailable { .. }
            | Self::NotConnected
            | Self::Connecting { .. } => None,
        }
    }

    /// Short status words for the page and the sidebar.
    pub fn status_label(&self) -> &'static str {
        match self {
            Self::Loading => "Checking…",
            Self::Unavailable { .. } => "Needs a client id",
            Self::NotConnected => "Not connected",
            Self::Connecting { .. } => "Waiting for you in your browser",
            Self::Connected(_) => "Connected",
            Self::NeedsSignIn(_) => "Needs you to sign in again",
        }
    }
}

/// A request from the Connections page to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleCommand {
    /// Read the saved account (sent once the runtime is ready).
    Load,
    /// Sign in, asking for every service. Also fixes [`GoogleState::NeedsSignIn`].
    Connect,
    /// Switch one service on or off. Turning on a service Google did not allow
    /// asks Google for just that one.
    SetService { service: GoogleService, on: bool },
    /// Stop waiting for the browser.
    Cancel,
    /// Sign out: Google forgets Sovra's access and the account entry is removed.
    /// The saved desktop client id stays.
    Disconnect,
    /// Keep a desktop client id (and optional secret) on this computer.
    SaveClient {
        client_id: String,
        client_secret: Option<ClientSecret>,
    },
}

/// A notification from the runtime about Google.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleEvent {
    Changed(GoogleState),
    /// Open Google's sign-in page in the person's browser (sent once per attempt).
    OpenSignIn {
        url: String,
    },
    /// Something to tell the person in plain words.
    Notice(String),
}

/// What the UI shows about Google, folded from [`GoogleEvent`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GoogleConnection {
    pub state: GoogleState,
    pub notice: Option<String>,
}

impl GoogleConnection {
    pub fn apply(&mut self, event: &GoogleEvent) {
        match event {
            GoogleEvent::Changed(state) => {
                let notice = match (&self.state, state) {
                    (GoogleState::Connecting { adding: None, .. }, GoogleState::Connected(a)) => {
                        Some(format!("Google is connected as {}.", a.email))
                    }
                    (
                        GoogleState::Connecting {
                            adding: Some(service),
                            ..
                        },
                        GoogleState::Connected(a),
                    ) if a.is_active(*service) => Some(format!(
                        "Sovra can now use your {}.",
                        service_info(*service).product
                    )),
                    (GoogleState::Connected(_), GoogleState::NotConnected) => {
                        Some("Google is disconnected. Sovra can no longer use it.".into())
                    }
                    (_, GoogleState::Connecting { .. }) => None,
                    _ => self.notice.take(),
                };
                self.notice = notice;
                self.state = state.clone();
            }
            GoogleEvent::OpenSignIn { .. } => {}
            GoogleEvent::Notice(message) => self.notice = Some(message.clone()),
        }
    }
}

/// Why a Google call failed, in words the person can act on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GoogleError {
    /// The saved sign-in no longer works (revoked, expired or password changed).
    #[error("{0}")]
    SignInNeeded(String),
    /// The person stopped, or never finished, signing in.
    #[error("Sign-in was cancelled.")]
    Cancelled,
    /// Google allowed sign-in but not this service.
    #[error("{0}")]
    NotAllowed(String),
    #[error("{0}")]
    Failed(String),
}

#[cfg(test)]
mod tests;
