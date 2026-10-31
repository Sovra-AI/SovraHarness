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

/// Secret storage supplied by the composition root (OS keychain in release).
pub trait SecretVault: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// This build's Google Cloud "Desktop app" OAuth client. Google documents the
/// installed-app secret as not confidential, so it may ship in the binary.
#[derive(Clone, Default)]
pub struct ClientConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
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

/// In-memory sign-in. Access tokens are never persisted.
#[derive(Clone, Default)]
struct Session {
    refresh_token: String,
    scope: String,
    access: Option<oauth::AccessToken>,
    contacts_warmed: bool,
}

pub struct GoogleAdapter {
    config: ClientConfig,
    vault: Arc<dyn SecretVault>,
    http: reqwest::Client,
    session: Mutex<Option<Session>>,
}

impl std::fmt::Debug for GoogleAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAdapter")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl GoogleAdapter {
    pub fn new(config: ClientConfig, vault: Arc<dyn SecretVault>) -> Result<Self, GoogleError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("Sovra/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| {
                GoogleError::Failed("Sovra couldn't prepare its connection to Google.".into())
            })?;
        Ok(Self {
            config,
            vault,
            http,
            session: Mutex::new(None),
        })
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
        let refreshed = oauth::refresh(&self.http, &self.config, &current.refresh_token).await?;
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

fn signed_out() -> GoogleError {
    GoogleError::SignInNeeded("Connect Google on the Connections page first.".into())
}

fn vault_error(_: String) -> GoogleError {
    GoogleError::Failed("Sovra couldn't reach your computer's password store.".into())
}

#[async_trait::async_trait]
impl GoogleConnector for GoogleAdapter {
    fn unavailable_reason(&self) -> Option<String> {
        self.config.client_id.trim().is_empty().then(|| {
            "Google sign-in isn't set up in this copy of Sovra yet. A newer version will \
             include it."
                .into()
        })
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
        let pending = loopback::PendingSignIn::open(&self.config.client_id, &scopes, login_hint)?;
        Ok(Box::new(pending))
    }

    async fn complete_sign_in(
        &self,
        grant: AuthorizationGrant,
    ) -> Result<GoogleAccount, GoogleError> {
        let tokens = oauth::exchange(&self.http, &self.config, &grant).await?;
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
