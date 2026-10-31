//! Google's OAuth 2.0 endpoints for installed apps: token exchange, refresh,
//! revoke and the `OpenID` profile. Three form POSTs and one GET; no crate needed.

use std::time::{Duration, Instant};

use serde::Deserialize;
use sovra_core::google::{AuthorizationGrant, GoogleError};

use crate::ClientConfig;

pub(crate) const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// Refresh a minute early so a call never starts with a token about to lapse.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub(crate) struct AccessToken {
    pub(crate) secret: String,
    expires_at: Instant,
}

impl AccessToken {
    pub(crate) fn is_fresh(&self) -> bool {
        Instant::now() + EXPIRY_MARGIN < self.expires_at
    }
}

pub(crate) struct Tokens {
    pub(crate) access: AccessToken,
    pub(crate) refresh_token: Option<String>,
    pub(crate) scope: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

#[derive(Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
}

pub(crate) struct Profile {
    pub(crate) email: String,
    pub(crate) name: String,
}

fn with_client<'a>(
    config: &'a ClientConfig,
    mut form: Vec<(&'static str, &'a str)>,
) -> Vec<(&'static str, &'a str)> {
    form.push(("client_id", config.client_id.as_str()));
    if let Some(secret) = config.client_secret.as_deref().filter(|s| !s.is_empty()) {
        form.push(("client_secret", secret));
    }
    form
}

pub(crate) async fn exchange(
    http: &reqwest::Client,
    config: &ClientConfig,
    grant: &AuthorizationGrant,
) -> Result<Tokens, GoogleError> {
    let form = with_client(
        config,
        vec![
            ("grant_type", "authorization_code"),
            ("code", grant.code.as_str()),
            ("code_verifier", grant.verifier.as_str()),
            ("redirect_uri", grant.redirect_uri.as_str()),
        ],
    );
    let response = token_request(http, &form).await?;
    Ok(Tokens {
        access: AccessToken {
            secret: response.access_token,
            expires_at: Instant::now() + Duration::from_secs(response.expires_in),
        },
        refresh_token: response.refresh_token,
        scope: response.scope,
    })
}

pub(crate) async fn refresh(
    http: &reqwest::Client,
    config: &ClientConfig,
    refresh_token: &str,
) -> Result<AccessToken, GoogleError> {
    let form = with_client(
        config,
        vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    );
    let response = token_request(http, &form).await?;
    Ok(AccessToken {
        secret: response.access_token,
        expires_at: Instant::now() + Duration::from_secs(response.expires_in),
    })
}

async fn token_request(
    http: &reqwest::Client,
    form: &[(&'static str, &str)],
) -> Result<TokenResponse, GoogleError> {
    let response = http
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .map_err(|_| offline())?;
    let ok = response.status().is_success();
    let bytes = response.bytes().await.map_err(|_| offline())?;
    if ok {
        return serde_json::from_slice(&bytes)
            .map_err(|_| GoogleError::Failed("Google sent a sign-in Sovra couldn't read.".into()));
    }
    let error = serde_json::from_slice::<TokenError>(&bytes)
        .map(|e| e.error)
        .unwrap_or_default();
    tracing::warn!(%error, "Google token request was refused");
    Err(token_failure(&error))
}

/// `invalid_grant` covers every way a saved sign-in dies: revoked, unused for
/// six months, a Gmail password change, or a seven-day Testing-mode token.
fn token_failure(error: &str) -> GoogleError {
    match error {
        "invalid_grant" => GoogleError::SignInNeeded(
            "Google needs you to sign in again. Open Connections and press Sign in again.".into(),
        ),
        "invalid_client" | "unauthorized_client" => GoogleError::Failed(
            "This copy of Sovra isn't allowed to sign in to Google. Please update Sovra.".into(),
        ),
        _ => GoogleError::Failed("Google couldn't finish signing in. Please try again.".into()),
    }
}

fn offline() -> GoogleError {
    GoogleError::Failed("Sovra couldn't reach Google. Check your internet connection.".into())
}

pub(crate) async fn profile(http: &reqwest::Client, access: &str) -> Result<Profile, GoogleError> {
    #[derive(Deserialize)]
    struct UserInfo {
        #[serde(default)]
        email: String,
        #[serde(default)]
        name: String,
    }
    let info: UserInfo = http
        .get(USERINFO_URL)
        .bearer_auth(access)
        .send()
        .await
        .map_err(|_| offline())?
        .error_for_status()
        .map_err(|_| GoogleError::Failed("Google didn't say which account signed in.".into()))?
        .json()
        .await
        .map_err(|_| GoogleError::Failed("Google didn't say which account signed in.".into()))?;
    Ok(Profile {
        email: info.email,
        name: info.name,
    })
}

/// Revoking the refresh token removes every scope Google granted Sovra.
pub(crate) async fn revoke(http: &reqwest::Client, token: &str) -> Result<(), GoogleError> {
    let response = http
        .post(REVOKE_URL)
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|_| offline())?;
    // 400 means the token was already invalid: Google has forgotten it either way.
    if response.status().is_success() || response.status() == reqwest::StatusCode::BAD_REQUEST {
        Ok(())
    } else {
        Err(GoogleError::Failed(
            "Google didn't confirm. You can also remove Sovra at myaccount.google.com/connections."
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_sign_ins_ask_the_person_to_sign_in_again() {
        assert!(matches!(
            token_failure("invalid_grant"),
            GoogleError::SignInNeeded(_)
        ));
        assert!(matches!(
            token_failure("invalid_client"),
            GoogleError::Failed(_)
        ));
    }

    #[test]
    fn the_client_secret_is_sent_only_when_configured() {
        let none = ClientConfig {
            client_id: "id".into(),
            client_secret: None,
        };
        assert_eq!(with_client(&none, vec![]), vec![("client_id", "id")]);
        let some = ClientConfig {
            client_id: "id".into(),
            client_secret: Some("s".into()),
        };
        assert_eq!(with_client(&some, vec![]).len(), 2);
    }
}
