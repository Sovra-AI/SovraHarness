//! Google consent parameters over the shared native browser callback.

use sovra_core::google::{AuthorizationGrant, GoogleError, GoogleSignIn, SignInCanceller};
use sovra_core::oauth::{NativeSignIn, OAuthError};
use sovra_oauth::{LoopbackSignIn, RedirectHost};

#[derive(Debug)]
pub(crate) struct PendingSignIn(LoopbackSignIn);

pub(crate) fn consent_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    state: &str,
    challenge: &str,
    login_hint: Option<&str>,
) -> String {
    let mut url = url::Url::parse(crate::oauth::AUTH_URL).expect("the auth URL is a constant");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &scopes.join(" "))
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline")
        // Always show the checkboxes, so turning a service back on can be granted.
        .append_pair("prompt", "consent");
    if let Some(hint) = login_hint.filter(|h| !h.is_empty()) {
        url.query_pairs_mut().append_pair("login_hint", hint);
    }
    url.into()
}

impl PendingSignIn {
    pub(crate) fn open(
        client_id: &str,
        scopes: &[&str],
        login_hint: Option<&str>,
    ) -> Result<Self, GoogleError> {
        LoopbackSignIn::open(RedirectHost::IpV4, |redirect, state, challenge| {
            consent_url(client_id, redirect, scopes, state, challenge, login_hint)
        })
        .map(Self)
        .map_err(google_error)
    }
}

fn google_error(error: OAuthError) -> GoogleError {
    match error {
        OAuthError::Cancelled => GoogleError::Cancelled,
        OAuthError::Denied => GoogleError::NotAllowed(
            "Google sign-in wasn't allowed, so nothing was connected.".into(),
        ),
        OAuthError::Failed(message) => GoogleError::Failed(message),
    }
}

impl GoogleSignIn for PendingSignIn {
    fn url(&self) -> &str {
        self.0.url()
    }
    fn canceller(&self) -> SignInCanceller {
        self.0.canceller()
    }
    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, GoogleError> {
        Box::new(self.0).wait().map_err(google_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_url_asks_for_pkce_offline_access_and_every_scope() {
        let url = consent_url(
            "client.apps.googleusercontent.com",
            "http://127.0.0.1:5000",
            &["openid", "email"],
            "st",
            "ch",
            Some("sam@example.com"),
        );
        let parsed = url::Url::parse(&url).expect("valid URL");
        let get = |key: &str| {
            parsed
                .query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert_eq!(get("scope").as_deref(), Some("openid email"));
        assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(
            get("redirect_uri").as_deref(),
            Some("http://127.0.0.1:5000")
        );
        assert_eq!(get("login_hint").as_deref(), Some("sam@example.com"));
        assert_eq!(get("state").as_deref(), Some("st"));
    }
}
