//! GitHub adapter: sign-in with the OAuth device flow (the person types a short code
//! on GitHub's own page) or a key the person pasted, the access kept in the vault the
//! composition root supplies, and a thin bounded REST client for the handful of
//! GitHub endpoints the agent tools use. Implements
//! `sovra_core::github::GitHubConnector`.
//!
//! Hand-rolled on the workspace's reqwest + rustls rather than an SDK crate: we use
//! about ten endpoints, and the SDKs bring their own HTTP stacks. See `docs/github.md`.

mod api;
mod http;

use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sovra_core::github::{
    DeviceAnswer, DeviceCode, DevicePoll, GitHubAccount, GitHubConnector, GitHubError, GitHubKey,
    GitHubRequest, SCOPES, device_answer,
};
pub use sovra_core::secrets::SecretVault;

/// The keychain entry name for the connected GitHub account.
pub const VAULT_KEY: &str = "connection.github";

/// Where GitHub lives. Tests point these at a local stand-in.
#[derive(Debug, Clone)]
pub struct Endpoints {
    /// `https://github.com`: the sign-in pages.
    pub web: String,
    /// `https://api.github.com`: the REST API.
    pub api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            web: "https://github.com".into(),
            api: "https://api.github.com".into(),
        }
    }
}

/// What the vault holds: the account and its access, as one JSON entry.
#[derive(Serialize, Deserialize)]
struct SavedSession {
    token: String,
    account: GitHubAccount,
}

pub struct GitHubAdapter {
    /// The public client id of Sovra's GitHub OAuth App, when this copy has one.
    client_id: Option<String>,
    endpoints: Endpoints,
    vault: Arc<dyn SecretVault>,
    http: reqwest::Client,
    /// The access in force for tool calls.
    session: Mutex<Option<String>>,
    /// A freshly granted access, waiting for [`GitHubConnector::save`].
    pending: Mutex<Option<String>>,
}

impl std::fmt::Debug for GitHubAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubAdapter")
            .field("client_id", &self.client_id)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl GitHubAdapter {
    /// `client_id` is the public id of a GitHub OAuth App with device flow switched on;
    /// a blank id means only pasted keys work.
    pub fn new(
        client_id: Option<String>,
        endpoints: Endpoints,
        vault: Arc<dyn SecretVault>,
    ) -> Result<Self, GitHubError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("Sovra/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| {
                GitHubError::Failed("Sovra couldn't prepare its connection to GitHub.".into())
            })?;
        Ok(Self {
            client_id: client_id
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty()),
            endpoints,
            vault,
            http,
            session: Mutex::new(None),
            pending: Mutex::new(None),
        })
    }

    fn token(&self) -> Result<String, GitHubError> {
        self.session
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| GitHubError::SignInNeeded(sovra_core::github::SIGN_IN_AGAIN.into()))
    }

    /// Who `token` belongs to.
    async fn whoami(&self, token: &str) -> Result<GitHubAccount, http::CallError> {
        let user = http::read_json(
            self.http
                .get(format!("{}/user", self.endpoints.api))
                .headers(http::api_headers(token)),
        )
        .await?;
        let login = user["login"].as_str().unwrap_or_default().to_owned();
        if login.is_empty() {
            return Err(http::CallError::Failed(
                "GitHub sent an answer Sovra couldn't read.".into(),
            ));
        }
        Ok(GitHubAccount {
            login,
            name: user["name"].as_str().unwrap_or_default().to_owned(),
        })
    }

    async fn post_form(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value, GitHubError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        let request = self
            .http
            .post(format!("{}{path}", self.endpoints.web))
            .header(reqwest::header::ACCEPT, "application/json")
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(body);
        // GitHub answers sign-in problems with 200 and an `error` field, so only
        // transport and size failures surface here.
        http::read_json(request)
            .await
            .map_err(http::CallError::into_github)
    }

    fn client_id(&self) -> Result<&str, GitHubError> {
        self.client_id.as_deref().ok_or_else(|| {
            GitHubError::Failed(
                "This copy of Sovra can't sign in on GitHub's page yet. Connect with a key \
                 instead."
                    .into(),
            )
        })
    }

    fn hold(&self, token: String) {
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = Some(token);
    }
}

#[async_trait::async_trait]
impl GitHubConnector for GitHubAdapter {
    fn device_sign_in(&self) -> bool {
        self.client_id.is_some()
    }

    fn restore(&self) -> Result<Option<GitHubAccount>, GitHubError> {
        let json = self.vault.get(VAULT_KEY).map_err(|_| {
            GitHubError::Failed("Sovra couldn't read its saved GitHub connection.".into())
        })?;
        let Some(json) = json else {
            return Ok(None);
        };
        let Ok(saved) = serde_json::from_str::<SavedSession>(&json) else {
            tracing::warn!("saved GitHub connection could not be read; ignoring it");
            return Ok(None);
        };
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = Some(saved.token);
        Ok(Some(saved.account))
    }

    async fn start_device_sign_in(&self) -> Result<DeviceCode, GitHubError> {
        let client_id = self.client_id()?.to_owned();
        let answer = self
            .post_form(
                "/login/device/code",
                &[("client_id", client_id.as_str()), ("scope", SCOPES)],
            )
            .await?;
        let text = |key: &str| answer[key].as_str().unwrap_or_default().to_owned();
        let device = DeviceCode {
            device_code: text("device_code"),
            user_code: text("user_code"),
            verification_uri: text("verification_uri"),
            interval_secs: answer["interval"].as_u64().unwrap_or(5),
            expires_in_secs: answer["expires_in"].as_u64().unwrap_or(900),
        };
        let page_ok = device.verification_uri.starts_with("https://");
        if device.device_code.is_empty() || device.user_code.is_empty() || !page_ok {
            tracing::warn!(error = %text("error"), "github device code refused");
            return Err(GitHubError::Failed(
                "GitHub didn't start the sign-in. Try again, or connect with a key instead.".into(),
            ));
        }
        Ok(device)
    }

    async fn poll_device_sign_in(&self, device: &DeviceCode) -> Result<DevicePoll, GitHubError> {
        let client_id = self.client_id()?.to_owned();
        let answer = self
            .post_form(
                "/login/oauth/access_token",
                &[
                    ("client_id", client_id.as_str()),
                    ("device_code", device.device_code.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ],
            )
            .await?;
        if let Some(token) = answer["access_token"].as_str().filter(|t| !t.is_empty()) {
            let account = self
                .whoami(token)
                .await
                .map_err(http::CallError::into_github)?;
            self.hold(token.to_owned());
            return Ok(DevicePoll::Done(account));
        }
        let error = answer["error"].as_str().unwrap_or_default();
        match device_answer(error) {
            DeviceAnswer::Pending => Ok(DevicePoll::Waiting),
            DeviceAnswer::SlowDown => Ok(DevicePoll::SlowDown),
            DeviceAnswer::Expired => Err(GitHubError::Failed(
                "The code ran out of time. Press Connect GitHub to get a new one.".into(),
            )),
            DeviceAnswer::Denied => Err(GitHubError::Failed(
                "You chose not to let Sovra use GitHub, so nothing was connected.".into(),
            )),
            DeviceAnswer::Failed => {
                tracing::warn!(%error, "github device sign-in failed");
                Err(GitHubError::Failed(
                    "GitHub couldn't finish signing in. Try again, or connect with a key \
                     instead."
                        .into(),
                ))
            }
        }
    }

    async fn check_key(&self, key: &GitHubKey) -> Result<GitHubAccount, GitHubError> {
        match self.whoami(key.as_str()).await {
            Ok(account) => {
                self.hold(key.as_str().to_owned());
                Ok(account)
            }
            Err(http::CallError::Unauthorized) => Err(GitHubError::Failed(
                "GitHub didn't accept that key. Check you copied all of it, then try again.".into(),
            )),
            Err(error) => Err(error.into_github()),
        }
    }

    fn save(&self, account: &GitHubAccount) -> Result<(), GitHubError> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let token = match pending {
            Some(token) => token,
            None => self.token()?,
        };
        let json = serde_json::to_string(&SavedSession {
            token: token.clone(),
            account: account.clone(),
        })
        .map_err(|_| GitHubError::Failed("Sovra couldn't save the GitHub connection.".into()))?;
        // In memory first: the connection works for this run even if the keychain refuses.
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = Some(token);
        self.vault.set(VAULT_KEY, &json).map_err(|_| {
            GitHubError::Failed(
                "GitHub is connected, but Sovra couldn't keep it for next time.".into(),
            )
        })
    }

    fn forget(&self) -> Result<(), GitHubError> {
        *self.session.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *self.pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.vault.delete(VAULT_KEY).map_err(|_| {
            GitHubError::Failed("Sovra couldn't remove the saved GitHub connection.".into())
        })
    }

    async fn call(&self, request: GitHubRequest) -> Result<Value, GitHubError> {
        let token = self.token()?;
        api::call(&self.http, &self.endpoints.api, &token, request)
            .await
            .map_err(http::CallError::into_github)
    }
}

#[cfg(test)]
mod tests;
