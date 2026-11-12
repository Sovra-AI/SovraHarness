//! Google adapter: desktop sign-in (system browser → single-use `127.0.0.1`
//! listener, PKCE S256), refresh tokens in the vault the composition root
//! supplies, and thin bounded REST clients for Gmail, Calendar, Drive, People
//! and Tasks. Implements `sovra_core::google::GoogleConnector`.
//!
//! Hand-rolled on the workspace's reqwest + rustls rather than generated Google
//! crates: we use about fifteen endpoints, and the generated crates bring a
//! second HTTP/TLS stack. See `docs/google.md`.

mod api;
mod http;
mod loopback;
mod mail;
mod oauth;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use sovra_core::google::{
    AuthorizationGrant, GoogleAccount, GoogleConnector, GoogleError, GoogleRequest, GoogleService,
    GoogleSignIn,
};

/// The keychain entry name for the signed-in Google account.
pub const VAULT_KEY: &str = "connection.google";

/// The keychain entry for the desktop OAuth client. Separate from [`VAULT_KEY`]
/// so signing out does not forget the client id.
pub const CLIENT_VAULT_KEY: &str = "connection.google.client";

/// Secret storage supplied by the composition root (OS keychain in release).
/// Shared with the GitHub adapter through `sovra-core`.
pub use sovra_core::secrets::SecretVault;

/// This build's Google Cloud "Desktop app" OAuth client. Google documents the
/// installed-app secret as not confidential, so it may ship in the binary.
#[derive(Clone, Default)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Where a desktop client may already be configured, before any id saved on
/// this computer. Empty ids are ignored.
#[derive(Clone, Default)]
pub struct ClientSources {
    pub environment: Option<ClientConfig>,
    pub baked: Option<ClientConfig>,
}

impl std::fmt::Debug for ClientSources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSources")
            .field("environment", &self.environment)
            .field("baked", &self.baked)
            .finish()
    }
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// What the vault holds: the account and the refresh token, as one JSON entry.
#[derive(Serialize, Deserialize)]
struct SavedSession {
    refresh_token: String,
    scope: String,
    account: GoogleAccount,
}

/// The desktop client saved on this computer. The secret stays in this JSON,
/// inside the vault, and is never logged.
#[derive(Serialize, Deserialize)]
struct SavedDesktopClient {
    client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
}

/// In-memory sign-in. Access tokens are never persisted.
#[derive(Clone, Default)]
struct Session {
    refresh_token: String,
    scope: String,
    access: Option<oauth::AccessToken>,
    contacts_warmed: bool,
}

pub struct GoogleAdapter {
    environment: Option<ClientConfig>,
    baked: Option<ClientConfig>,
    vault: Arc<dyn SecretVault>,
    http: reqwest::Client,
    session: Mutex<Option<Session>>,
}

impl std::fmt::Debug for GoogleAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAdapter")
            .field("environment", &self.environment)
            .field("baked", &self.baked)
            .finish_non_exhaustive()
    }
}

impl GoogleAdapter {
    pub fn new(sources: ClientSources, vault: Arc<dyn SecretVault>) -> Result<Self, GoogleError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("Sovra/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| {
                GoogleError::Failed("Sovra couldn't prepare its connection to Google.".into())
            })?;
        Ok(Self {
            environment: sources.environment,
            baked: sources.baked,
            vault,
            http,
            session: Mutex::new(None),
        })
    }

    fn saved_client(&self) -> Option<sovra_core::google::DesktopClient> {
        let json = match self.vault.get(CLIENT_VAULT_KEY) {
            Ok(Some(json)) => json,
            Ok(None) => return None,
            Err(_) => {
                tracing::warn!("could not read the saved Google client id");
                return None;
            }
        };
        if let Ok(saved) = serde_json::from_str::<SavedDesktopClient>(&json) {
            sovra_core::google::normalize_desktop_client(sovra_core::google::DesktopClient {
                client_id: saved.client_id,
                client_secret: saved.client_secret,
            })
        } else {
            // The body can hold the secret, so the warning stays generic.
            tracing::warn!("saved Google client id was unreadable");
            None
        }
    }

    fn client_config(&self) -> Result<ClientConfig, GoogleError> {
        self.desktop_client()
            .map(|client| ClientConfig {
                client_id: client.client_id,
                client_secret: client.client_secret,
            })
            .ok_or_else(|| GoogleError::Failed(sovra_core::google::DESKTOP_CLIENT_NEEDED.into()))
    }

    fn session(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        // A panic while holding the lock leaves plain data, never a broken invariant.
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A valid access token, refreshing it when it is missing or about to expire.
    async fn access_token(&self) -> Result<String, GoogleError> {
        let current = self.session().clone().ok_or_else(signed_out)?;
        if let Some(token) = current.access.as_ref().filter(|t| t.is_fresh()) {
            return Ok(token.secret.clone());
        }
        let config = self.client_config()?;
        let refreshed = oauth::refresh(&self.http, &config, &current.refresh_token).await?;
        let secret = refreshed.secret.clone();
        if let Some(session) = self.session().as_mut() {
            session.access = Some(refreshed);
        }
        Ok(secret)
    }

    /// Forgets the cached access token so the next call refreshes it.
    fn expire_access(&self) {
        if let Some(session) = self.session().as_mut() {
            session.access = None;
        }
    }

    fn take_contacts_warmup(&self) -> bool {
        self.session()
            .as_mut()
            .is_some_and(|session| !std::mem::replace(&mut session.contacts_warmed, true))
    }
}

fn as_desktop(config: &ClientConfig) -> Option<sovra_core::google::DesktopClient> {
    sovra_core::google::normalize_desktop_client(sovra_core::google::DesktopClient {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
    })
}

fn signed_out() -> GoogleError {
    GoogleError::SignInNeeded("Connect Google on the Connections page first.".into())
}

fn vault_error(_: String) -> GoogleError {
    GoogleError::Failed("Sovra couldn't reach your computer's password store.".into())
}

#[async_trait::async_trait]
impl GoogleConnector for GoogleAdapter {
    fn unavailable_reason(&self) -> Option<String> {
        self.desktop_client()
            .is_none()
            .then(|| sovra_core::google::DESKTOP_CLIENT_NEEDED.into())
    }

    fn desktop_client(&self) -> Option<sovra_core::google::DesktopClient> {
        sovra_core::google::client_in_force(
            self.saved_client(),
            self.environment.as_ref().and_then(as_desktop),
            self.baked.as_ref().and_then(as_desktop),
        )
    }

    fn save_desktop_client(
        &self,
        client_id: &str,
        client_secret: Option<String>,
    ) -> Result<(), GoogleError> {
        let Some(client) =
            sovra_core::google::normalize_desktop_client(sovra_core::google::DesktopClient {
                client_id: client_id.to_owned(),
                client_secret,
            })
        else {
            return Err(GoogleError::Failed(
                "Enter the desktop client id from your Google Cloud project.".into(),
            ));
        };
        let json = serde_json::to_string(&SavedDesktopClient {
            client_id: client.client_id,
            client_secret: client.client_secret,
        })
        .map_err(|_| GoogleError::Failed("Sovra couldn't save the Google client id.".into()))?;
        self.vault.set(CLIENT_VAULT_KEY, &json).map_err(vault_error)
    }

    fn restore(&self) -> Result<Option<GoogleAccount>, GoogleError> {
        let Some(json) = self.vault.get(VAULT_KEY).map_err(vault_error)? else {
            return Ok(None);
        };
        let Ok(saved) = serde_json::from_str::<SavedSession>(&json) else {
            tracing::warn!("saved Google sign-in was unreadable; signing out");
            return Ok(None);
        };
        *self.session() = Some(Session {
            refresh_token: saved.refresh_token,
            scope: saved.scope,
            ..Session::default()
        });
        Ok(Some(saved.account))
    }

    fn begin_sign_in(
        &self,
        services: &BTreeSet<GoogleService>,
        login_hint: Option<&str>,
    ) -> Result<Box<dyn GoogleSignIn>, GoogleError> {
        if let Some(reason) = self.unavailable_reason() {
            return Err(GoogleError::Failed(reason));
        }
        let services: Vec<GoogleService> = services.iter().copied().collect();
        let scopes = sovra_core::google::scopes_for(&services);
        let config = self.client_config()?;
        let pending = loopback::PendingSignIn::open(&config.client_id, &scopes, login_hint)?;
        Ok(Box::new(pending))
    }

    async fn complete_sign_in(
        &self,
        grant: AuthorizationGrant,
    ) -> Result<GoogleAccount, GoogleError> {
        let config = self.client_config()?;
        let tokens = oauth::exchange(&self.http, &config, &grant).await?;
        let profile = oauth::profile(&self.http, &tokens.access.secret).await?;
        let previous = self.session().clone();
        // Google returns a refresh token on every consent for installed apps; keep
        // the old one if it ever does not.
        let refresh_token = tokens
            .refresh_token
            .or_else(|| previous.map(|p| p.refresh_token))
            .ok_or_else(|| {
                GoogleError::Failed(
                    "Google didn't let Sovra stay signed in. Please try again.".into(),
                )
            })?;
        let granted = sovra_core::google::granted_services(&tokens.scope);
        *self.session() = Some(Session {
            refresh_token,
            scope: tokens.scope,
            access: Some(tokens.access),
            contacts_warmed: false,
        });
        Ok(GoogleAccount {
            email: profile.email,
            name: profile.name,
            granted,
            turned_off: BTreeSet::new(),
        })
    }

    fn save(&self, account: &GoogleAccount) -> Result<(), GoogleError> {
        let session = self.session().clone().ok_or_else(signed_out)?;
        let saved = SavedSession {
            refresh_token: session.refresh_token,
            scope: session.scope,
            account: account.clone(),
        };
        let json = serde_json::to_string(&saved)
            .map_err(|_| GoogleError::Failed("Sovra couldn't save your Google sign-in.".into()))?;
        self.vault.set(VAULT_KEY, &json).map_err(vault_error)
    }

    async fn revoke(&self) -> Result<(), GoogleError> {
        let Some(session) = self.session().clone() else {
            return Ok(());
        };
        oauth::revoke(&self.http, &session.refresh_token).await
    }

    fn forget(&self) -> Result<(), GoogleError> {
        *self.session() = None;
        // The account entry only. The desktop client id stays for the next sign-in.
        self.vault.delete(VAULT_KEY).map_err(vault_error)
    }

    async fn call(&self, request: GoogleRequest) -> Result<serde_json::Value, GoogleError> {
        let token = self.access_token().await?;
        match api::call(self, &token, &request).await {
            // An access token can be revoked before it expires; refresh once.
            Err(http::CallError::Unauthorized) => {
                self.expire_access();
                let token = self.access_token().await?;
                api::call(self, &token, &request)
                    .await
                    .map_err(|error| error.into_google(&request))
            }
            other => other.map_err(|error| error.into_google(&request)),
        }
    }
}

#[cfg(test)]
mod tests;
