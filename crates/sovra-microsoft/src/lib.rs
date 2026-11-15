//! Microsoft 365 OAuth and Graph IO. Runtime drives futures and supplies the vault.

mod api;
mod http;
mod oauth;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use sovra_core::microsoft::{
    MicrosoftAccount, MicrosoftClient, MicrosoftConnector, MicrosoftError, MicrosoftOutput,
    MicrosoftRequest, MicrosoftService, MicrosoftSession, granted_services, scopes_for,
};
use sovra_core::oauth::{AuthorizationGrant, NativeSignIn};
use sovra_core::secrets::AsyncSecretVault;
use sovra_oauth::{LoopbackSignIn, RedirectHost};

const VAULT_KEY: &str = "connection.microsoft";
const CLIENT_KEY: &str = "connection.microsoft.client";

#[derive(Debug, Clone)]
struct Endpoints {
    authorize: url::Url,
    token: url::Url,
    graph: url::Url,
}

pub struct Microsoft {
    http: reqwest::Client,
    vault: Arc<dyn AsyncSecretVault>,
}

impl std::fmt::Debug for Microsoft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Microsoft").finish_non_exhaustive()
    }
}

impl Microsoft {
    pub fn new(vault: Arc<dyn AsyncSecretVault>) -> Result<Self, MicrosoftError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Sovra")
            .build()
            .map_err(|_| http::offline())?;
        Ok(Self { http, vault })
    }

    fn session(
        &self,
        stored: Stored,
        access: Option<oauth::AccessToken>,
    ) -> Arc<dyn MicrosoftSession> {
        Arc::new(Session {
            account: stored.account.clone(),
            config: stored.config.clone(),
            http: self.http.clone(),
            vault: self.vault.clone(),
            endpoints: Endpoints::for_client(&stored.config),
            state: Mutex::new(SessionState {
                stored,
                access,
                disabled: false,
                retry_at: None,
            }),
        })
    }
}

impl Endpoints {
    fn for_client(config: &MicrosoftClient) -> Self {
        let tenant = &config.tenant;
        Self {
            authorize: url::Url::parse(&format!(
                "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize"
            ))
            .expect("validated tenant"),
            token: url::Url::parse(&format!(
                "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
            ))
            .expect("validated tenant"),
            graph: url::Url::parse("https://graph.microsoft.com/v1.0/")
                .expect("constant Graph URL"),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    config: MicrosoftClient,
    refresh_token: String,
    account: MicrosoftAccount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: String,
    display_name: Option<String>,
    mail: Option<String>,
    user_principal_name: Option<String>,
}

#[async_trait::async_trait]
impl MicrosoftConnector for Microsoft {
    async fn load_client(&self) -> Result<Option<MicrosoftClient>, MicrosoftError> {
        let Some(value) = self.vault.get(CLIENT_KEY).await.map_err(vault_error)? else {
            return Ok(None);
        };
        let client: MicrosoftClient = serde_json::from_str(&value).map_err(|_| {
            MicrosoftError::Failed(
                "Your saved Microsoft setup could not be read. Enter your public IDs again.".into(),
            )
        })?;
        client.validate()?;
        Ok(Some(client))
    }

    async fn save_client(&self, client: &MicrosoftClient) -> Result<(), MicrosoftError> {
        client.validate()?;
        // Replacing a registration must never reuse an old registration's refresh token.
        // Corrupt local setup can be repaired by entering the public IDs again.
        if self.load_client().await.ok().flatten().as_ref() != Some(client) {
            self.vault.delete(VAULT_KEY).await.map_err(vault_error)?;
        }
        let value = serde_json::to_string(client).expect("public IDs serialize");
        self.vault
            .set(CLIENT_KEY, &value)
            .await
            .map_err(vault_error)
    }

    async fn forget_client(&self) -> Result<(), MicrosoftError> {
        self.vault.delete(VAULT_KEY).await.map_err(vault_error)?;
        self.vault.delete(CLIENT_KEY).await.map_err(vault_error)
    }

    fn begin_sign_in(
        &self,
        config: &MicrosoftClient,
        services: &BTreeSet<MicrosoftService>,
        login_hint: Option<&str>,
    ) -> Result<Box<dyn NativeSignIn>, MicrosoftError> {
        config.validate()?;
        let endpoints = Endpoints::for_client(config);
        let scopes = scopes_for(services);
        let pending =
            LoopbackSignIn::open(RedirectHost::Localhost, |redirect, state, challenge| {
                oauth::consent_url(
                    &endpoints.authorize,
                    config,
                    redirect,
                    state,
                    challenge,
                    &scopes,
                    login_hint,
                )
            })
            .map_err(|_| {
                MicrosoftError::Failed(
                    "Sovra couldn't start Microsoft sign-in. Please try again.".into(),
                )
            })?;
        Ok(Box::new(pending))
    }

    async fn restore(
        &self,
        config: &MicrosoftClient,
    ) -> Result<Option<Arc<dyn MicrosoftSession>>, MicrosoftError> {
        config.validate()?;
        let Some(secret) = self.vault.get(VAULT_KEY).await.map_err(vault_error)? else {
            return Ok(None);
        };
        let stored: Stored =
            serde_json::from_str(&secret).map_err(|_| MicrosoftError::SignInNeeded)?;
        if &stored.config != config {
            return Ok(None);
        }
        if stored.refresh_token.is_empty() || stored.account.id.is_empty() {
            return Err(MicrosoftError::SignInNeeded);
        }
        Ok(Some(self.session(stored, None)))
    }

    async fn complete_sign_in(
        &self,
        config: &MicrosoftClient,
        grant: AuthorizationGrant,
    ) -> Result<Arc<dyn MicrosoftSession>, MicrosoftError> {
        config.validate()?;
        let endpoints = Endpoints::for_client(config);
        let tokens = oauth::token_request(
            &self.http,
            &endpoints.token,
            &[
                ("client_id", &config.client_id),
                ("grant_type", "authorization_code"),
                ("code", &grant.code),
                ("code_verifier", &grant.verifier),
                ("redirect_uri", &grant.redirect_uri),
            ],
        )
        .await?;
        let access = tokens.access()?;
        let refresh_token = tokens
            .refresh_token
            .filter(|token| !token.is_empty())
            .ok_or(MicrosoftError::SignInNeeded)?;
        let mut profile_url = endpoints.graph.join("me").expect("relative constant");
        profile_url
            .query_pairs_mut()
            .append_pair("$select", "id,displayName,mail,userPrincipalName");
        let response = self
            .http
            .get(profile_url)
            .bearer_auth(&access.secret)
            .send()
            .await
            .map_err(|_| http::offline())?;
        let profile: Profile = http::json(response).await?;
        if profile.id.is_empty() {
            return Err(MicrosoftError::SignInNeeded);
        }
        let account = MicrosoftAccount {
            id: profile.id,
            name: profile.display_name.unwrap_or_default(),
            email: profile
                .mail
                .or(profile.user_principal_name)
                .unwrap_or_default(),
            granted: granted_services(&tokens.scope),
            turned_off: BTreeSet::new(),
        };
        Ok(self.session(
            Stored {
                config: config.clone(),
                refresh_token,
                account,
            },
            Some(access),
        ))
    }
}

struct SessionState {
    stored: Stored,
    access: Option<oauth::AccessToken>,
    disabled: bool,
    retry_at: Option<Instant>,
}

struct Session {
    account: MicrosoftAccount,
    config: MicrosoftClient,
    http: reqwest::Client,
    vault: Arc<dyn AsyncSecretVault>,
    endpoints: Endpoints,
    state: Mutex<SessionState>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MicrosoftSession(..)")
    }
}

impl Session {
    async fn persist(&self, stored: &Stored) -> Result<(), MicrosoftError> {
        let secret = serde_json::to_string(stored).map_err(|_| vault_error(String::new()))?;
        self.vault
            .set(VAULT_KEY, &secret)
            .await
            .map_err(vault_error)
    }

    async fn access(&self, state: &mut SessionState) -> Result<String, MicrosoftError> {
        if state.disabled {
            return Err(MicrosoftError::NotAllowed);
        }
        if let Some(access) = &state.access
            && access.fresh()
        {
            return Ok(access.secret.clone());
        }
        let scope = scopes_for(&state.stored.account.granted).join(" ");
        let tokens = oauth::token_request(
            &self.http,
            &self.endpoints.token,
            &[
                ("client_id", &self.config.client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", &state.stored.refresh_token),
                ("scope", &scope),
            ],
        )
        .await?;
        let access = tokens.access()?;
        if let Some(refresh) = tokens.refresh_token.filter(|token| !token.is_empty()) {
            state.stored.refresh_token = refresh;
        }
        if !tokens.scope.is_empty() {
            state.stored.account.granted = granted_services(&tokens.scope);
        }
        self.persist(&state.stored).await?;
        let secret = access.secret.clone();
        state.access = Some(access);
        Ok(secret)
    }
}

#[async_trait::async_trait]
impl MicrosoftSession for Session {
    fn account(&self) -> MicrosoftAccount {
        self.account.clone()
    }

    async fn save(&self, account: &MicrosoftAccount) -> Result<(), MicrosoftError> {
        let mut state = self.state.lock().await;
        if state.disabled || account.id != self.account.id {
            return Err(MicrosoftError::NotAllowed);
        }
        let stored = Stored {
            config: state.stored.config.clone(),
            refresh_token: state.stored.refresh_token.clone(),
            account: MicrosoftAccount {
                turned_off: account.turned_off.clone(),
                ..state.stored.account.clone()
            },
        };
        self.persist(&stored).await?;
        state.stored = stored;
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), MicrosoftError> {
        let mut state = self.state.lock().await;
        state.disabled = true;
        state.access = None;
        state.stored.refresh_token.clear();
        self.vault.delete(VAULT_KEY).await.map_err(vault_error)
    }

    async fn retire(&self) {
        let mut state = self.state.lock().await;
        state.disabled = true;
        state.access = None;
        state.stored.refresh_token.clear();
    }

    async fn call(&self, request: MicrosoftRequest) -> Result<MicrosoftOutput, MicrosoftError> {
        request.validate()?;
        // One lock covers refresh, its vault write, and the API call. Disconnect
        // waits for an in-flight operation, then permanently disables this session.
        let mut state = self.state.lock().await;
        if !state.stored.account.active(request.service()) {
            return Err(MicrosoftError::NotAllowed);
        }
        if let Some(until) = state.retry_at {
            let remaining = until.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                return Err(MicrosoftError::Throttled(
                    remaining.as_secs().saturating_add(1),
                ));
            }
            state.retry_at = None;
        }
        let access = match self.access(&mut state).await {
            Ok(access) => access,
            Err(error) => {
                remember_throttle(&mut state, &error);
                return Err(error);
            }
        };
        if !state.stored.account.active(request.service()) {
            // A refresh can return narrower grants. Reconnect so runtime replaces
            // its old permission snapshot with the permissions actually granted.
            return Err(MicrosoftError::SignInNeeded);
        }
        let result = api::call(&self.http, &self.endpoints.graph, &access, &request).await;
        if matches!(result, Err(MicrosoftError::SignInNeeded)) {
            state.access = None;
        }
        if let Err(error) = &result {
            remember_throttle(&mut state, error);
        }
        result
    }
}

fn remember_throttle(state: &mut SessionState, error: &MicrosoftError) {
    if let MicrosoftError::Throttled(seconds) = error {
        state.retry_at = Some(Instant::now() + Duration::from_secs(*seconds));
    }
}

fn vault_error(_error: String) -> MicrosoftError {
    MicrosoftError::Failed("Sovra couldn't update its saved Microsoft connection. Check this computer's credential store.".into())
}

#[cfg(test)]
mod tests;
