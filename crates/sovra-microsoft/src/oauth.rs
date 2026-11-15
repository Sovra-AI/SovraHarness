//! Microsoft public-client authorization and rotating refresh tokens.

use std::time::{Duration, Instant};

use serde::Deserialize;
use sovra_core::microsoft::{MicrosoftClient, MicrosoftError};

use crate::http;

pub(crate) struct AccessToken {
    pub(crate) secret: String,
    pub(crate) expires_at: Instant,
}

impl AccessToken {
    pub(crate) fn fresh(&self) -> bool {
        self.expires_at.saturating_duration_since(Instant::now()) > Duration::from_secs(60)
    }
}

#[derive(Deserialize)]
pub(crate) struct Tokens {
    pub(crate) access_token: String,
    pub(crate) refresh_token: Option<String>,
    pub(crate) expires_in: u64,
    #[serde(default)]
    pub(crate) scope: String,
    pub(crate) token_type: String,
}

impl Tokens {
    pub(crate) fn access(&self) -> Result<AccessToken, MicrosoftError> {
        if self.access_token.is_empty()
            || !self.token_type.eq_ignore_ascii_case("bearer")
            || self.expires_in == 0
        {
            return Err(MicrosoftError::SignInNeeded);
        }
        Ok(AccessToken {
            secret: self.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(self.expires_in.min(86_400)),
        })
    }
}

pub(crate) fn consent_url(
    endpoint: &url::Url,
    config: &MicrosoftClient,
    redirect: &str,
    state: &str,
    challenge: &str,
    scopes: &[&str],
    hint: Option<&str>,
) -> String {
    let mut url = endpoint.clone();
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", redirect)
        .append_pair("response_type", "code")
        .append_pair("response_mode", "query")
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    if let Some(hint) = hint.filter(|hint| !hint.is_empty()) {
        url.query_pairs_mut().append_pair("login_hint", hint);
    } else {
        url.query_pairs_mut()
            .append_pair("prompt", "select_account");
    }
    url.into()
}

#[derive(Deserialize, Default)]
struct Failure {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_codes: Vec<u64>,
}

pub(crate) async fn token_request(
    client: &reqwest::Client,
    endpoint: &url::Url,
    form: &[(&str, &str)],
) -> Result<Tokens, MicrosoftError> {
    let response = client
        .post(endpoint.clone())
        .form(form)
        .send()
        .await
        .map_err(|_| http::offline())?;
    parse_token_response(response).await
}

async fn parse_token_response(response: reqwest::Response) -> Result<Tokens, MicrosoftError> {
    if matches!(response.status().as_u16(), 429 | 503) {
        http::status(&response)?;
    }
    let success = response.status().is_success();
    let bytes = http::bytes(response, 64 * 1024).await?;
    if success {
        return serde_json::from_slice(&bytes).map_err(|_| {
            MicrosoftError::Failed("Microsoft sent an unreadable sign-in reply.".into())
        });
    }
    let failure: Failure = serde_json::from_slice(&bytes).unwrap_or_default();
    if failure
        .error_codes
        .iter()
        .any(|code| matches!(code, 90_093 | 90_094 | 900_941))
    {
        return Err(MicrosoftError::ApprovalNeeded);
    }
    Err(match failure.error.as_str() {
        "invalid_grant" | "interaction_required" | "login_required" | "consent_required" => {
            MicrosoftError::SignInNeeded
        }
        "access_denied" => MicrosoftError::NotAllowed,
        "invalid_client" | "unauthorized_client" => MicrosoftError::Failed(
            "Open Edit setup in Connections. Check your Application (client) ID, account type and the Mobile and desktop return address in Microsoft. No client secret is needed.".into(),
        ),
        _ => {
            MicrosoftError::Failed("Microsoft couldn't finish signing in. Please try again.".into())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_failures_are_actionable_and_never_echo_sensitive_bodies() {
        for (body, expected) in [
            (
                r#"{"error":"invalid_grant","error_description":"private token"}"#,
                MicrosoftError::SignInNeeded,
            ),
            (r#"{"error":"access_denied"}"#, MicrosoftError::NotAllowed),
            (
                r#"{"error":"access_denied","error_codes":[90094]}"#,
                MicrosoftError::ApprovalNeeded,
            ),
        ] {
            let response = ::http::Response::builder().status(400).body(body).unwrap();
            let error = futures::executor::block_on(parse_token_response(response.into()))
                .err()
                .unwrap();
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("private token"));
        }
        let response = ::http::Response::builder()
            .status(400)
            .body(r#"{"error":"invalid_client","error_description":"private registration detail"}"#)
            .unwrap();
        let error = futures::executor::block_on(parse_token_response(response.into()))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("Edit setup"));
        assert!(!error.contains("private registration detail"));
        let response = ::http::Response::builder()
            .status(429)
            .header("Retry-After", "90")
            .body("private body")
            .unwrap();
        assert_eq!(
            futures::executor::block_on(parse_token_response(response.into()))
                .err()
                .unwrap(),
            MicrosoftError::Throttled(90)
        );
    }
}
