//! Sign-in adapters: Argon2id password hashing and the Google OAuth 2.0
//! authorization-code + PKCE exchange for the RFC 8252 loopback flow.
//!
//! No sockets or tasks here: `sovra-runtime` owns the loopback listener and
//! drives [`GoogleAuthFlow`]. Everything secret (PKCE verifier, CSRF state,
//! refresh tokens) stays out of `Debug` output and logs.

use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse as _, TokenUrl,
};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
/// Google's userinfo answer is a small JSON object; anything bigger is wrong.
const MAX_USERINFO_BYTES: usize = 64 * 1024;
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// What can go wrong while signing in. Messages are safe to show and log:
/// they never carry passwords, codes or tokens.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Could not protect the password on this computer.")]
    Hashing,
    #[error("Google needs a desktop client id. Add one in Settings, under Connections.")]
    NotConfigured,
    #[error("Google sign-in did not finish: {0}")]
    Google(String),
}

/// Hashes a password with Argon2id and the crate's OWASP-aligned defaults
/// (m=19 MiB, t=2, p=1). The PHC string embeds the salt and parameters, so
/// future parameter upgrades verify old hashes unchanged.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    use argon2::password_hash::PasswordHasher as _;
    argon2::Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Hashing)
}

/// Checks a password against a stored PHC string. An unreadable hash counts
/// as a failed check, never a pass.
pub fn verify_password(phc: &str, password: &str) -> bool {
    use argon2::password_hash::{PasswordVerifier as _, phc::PasswordHash};
    PasswordHash::new(phc).is_ok_and(|parsed| {
        argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

/// Google OAuth client settings, read once by the composition root. A desktop
/// app's client secret is not confidential (RFC 8252 §8.5), but it still stays
/// out of `Debug` output.
#[derive(Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for GoogleOAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleOAuthConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "…"))
            .finish()
    }
}

/// The person Google vouched for, plus the refresh token destined for the
/// OS keychain. Never serialized; `Debug` redacts the token.
pub struct GoogleIdentity {
    pub email: String,
    pub full_name: String,
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for GoogleIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleIdentity")
            .field("email", &self.email)
            .field("full_name", &self.full_name)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "…"))
            .finish()
    }
}

/// One Google sign-in attempt: fresh PKCE verifier and CSRF state bound to a
/// loopback redirect on `127.0.0.1:{port}` (RFC 8252 §7.3).
pub struct GoogleAuthFlow {
    config: GoogleOAuthConfig,
    redirect_url: String,
    verifier: PkceCodeVerifier,
    state: CsrfToken,
    auth_url: String,
}

/// A one-time sign-in verifier and matching state. `Debug` hides the verifier.
#[derive(Clone)]
pub struct SignInProof {
    verifier: String,
    state: String,
}

impl SignInProof {
    /// Random verifier from the system generator, so this works on every desktop.
    pub fn generate() -> Self {
        let (_, verifier) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random();
        Self {
            verifier: verifier.secret().to_owned(),
            state: state.secret().to_owned(),
        }
    }

    pub fn verifier(&self) -> &str {
        &self.verifier
    }

    pub fn state(&self) -> &str {
        &self.state
    }
}

impl std::fmt::Debug for SignInProof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignInProof")
            .field("verifier", &"…")
            .field("state", &self.state)
            .finish()
    }
}

impl std::fmt::Debug for GoogleAuthFlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAuthFlow")
            .field("redirect_url", &self.redirect_url)
            .finish_non_exhaustive()
    }
}

impl GoogleAuthFlow {
    /// Builds the consent URL. `port` is the OS-assigned loopback port the
    /// runtime is already listening on; RFC 8252 requires the literal
    /// `127.0.0.1`, never `localhost`.
    pub fn begin(config: GoogleOAuthConfig, port: u16) -> Result<Self, AuthError> {
        if config.client_id.trim().is_empty() {
            return Err(AuthError::NotConfigured);
        }
        let redirect_url = format!("http://127.0.0.1:{port}/callback");
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (auth_url, state) = BasicClient::new(ClientId::new(config.client_id.clone()))
            .set_auth_uri(auth_uri()?)
            .set_redirect_uri(
                RedirectUrl::new(redirect_url.clone())
                    .map_err(|_| AuthError::Google("invalid redirect URL".into()))?,
            )
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new("openid".into()))
            .add_scope(Scope::new("email".into()))
            .add_scope(Scope::new("profile".into()))
            .set_pkce_challenge(challenge)
            .url();
        Ok(Self {
            config,
            redirect_url,
            verifier,
            state,
            auth_url: auth_url.to_string(),
        })
    }

    /// The Google consent page to open in the person's browser.
    pub fn auth_url(&self) -> &str {
        &self.auth_url
    }

    /// Compares the `state` the browser brought back with this attempt's,
    /// in constant time (CSRF protection, RFC 8252 §8.9).
    pub fn state_matches(&self, returned_state: &str) -> bool {
        sovra_core::auth::constant_time_str_eq(self.state.secret(), returned_state)
    }

    /// Redeems the authorization code (PKCE-bound) and asks Google who the
    /// person is. Only verified email addresses are accepted.
    pub async fn exchange(self, code: String) -> Result<GoogleIdentity, AuthError> {
        // No redirects on the token endpoint: a redirect here is an attack.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|_| AuthError::Google("could not start the secure client".into()))?;
        let client = BasicClient::new(ClientId::new(self.config.client_id))
            .set_auth_uri(auth_uri()?)
            .set_token_uri(
                TokenUrl::new(GOOGLE_TOKEN_URL.to_owned()).map_err(|_| AuthError::NotConfigured)?,
            )
            .set_redirect_uri(
                RedirectUrl::new(self.redirect_url)
                    .map_err(|_| AuthError::Google("invalid redirect URL".into()))?,
            );
        let client = match self.config.client_secret {
            Some(secret) => client.set_client_secret(ClientSecret::new(secret)),
            None => client,
        };
        let token = client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(self.verifier)
            .request_async(&http)
            .await
            .map_err(|error| AuthError::Google(error.to_string()))?;
        let identity = fetch_identity(&http, token.access_token().secret()).await?;
        Ok(GoogleIdentity {
            refresh_token: token.refresh_token().map(|t| t.secret().clone()),
            ..identity
        })
    }
}

fn auth_uri() -> Result<AuthUrl, AuthError> {
    AuthUrl::new(GOOGLE_AUTH_URL.to_owned()).map_err(|_| AuthError::NotConfigured)
}

#[derive(serde::Deserialize)]
struct UserInfo {
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    given_name: Option<String>,
}

async fn fetch_identity(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<GoogleIdentity, AuthError> {
    let response = http
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| AuthError::Google("could not reach Google to confirm who you are".into()))?;
    if !response.status().is_success() {
        return Err(AuthError::Google(
            "Google did not confirm the account".into(),
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AuthError::Google("the reply from Google was interrupted".into()))?;
    if bytes.len() > MAX_USERINFO_BYTES {
        return Err(AuthError::Google(
            "the reply from Google was too large".into(),
        ));
    }
    let info: UserInfo = serde_json::from_slice(&bytes)
        .map_err(|_| AuthError::Google("the reply from Google was unreadable".into()))?;
    let email = info
        .email
        .filter(|email| !email.trim().is_empty())
        .ok_or_else(|| AuthError::Google("Google did not share an email address".into()))?;
    if info.email_verified == Some(false) {
        return Err(AuthError::Google(
            "this Google account's email address is not verified".into(),
        ));
    }
    let full_name = info
        .name
        .or(info.given_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or(&email).to_owned());
    Ok(GoogleIdentity {
        email: email.trim().to_lowercase(),
        full_name,
        refresh_token: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip_rejects_wrong_and_corrupt_input() {
        let phc = hash_password("correct horse battery staple").expect("hashing works");
        assert!(phc.starts_with("$argon2id$"), "unexpected PHC: {phc}");
        assert!(verify_password(&phc, "correct horse battery staple"));
        assert!(!verify_password(&phc, "wrong password"));
        assert!(!verify_password("not-a-phc-string", "anything"));
        let second = hash_password("correct horse battery staple").expect("hashing works");
        assert_ne!(phc, second, "salts must be random per hash");
    }

    #[test]
    fn google_flow_builds_a_pkce_consent_url_and_checks_state() {
        let flow = GoogleAuthFlow::begin(
            GoogleOAuthConfig {
                client_id: "client-123.apps.googleusercontent.com".into(),
                client_secret: None,
            },
            43210,
        )
        .expect("flow starts");
        let url = flow.auth_url();
        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("code_challenge_method=S256"), "{url}");
        assert!(
            url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A43210%2Fcallback"),
            "{url}"
        );
        assert!(url.contains("scope=openid+email+profile"), "{url}");
        assert!(url.contains("state="), "{url}");

        let state = url
            .split("state=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .expect("state param present");
        assert!(flow.state_matches(state));
        assert!(!flow.state_matches("forged-state"));

        let empty = GoogleAuthFlow::begin(
            GoogleOAuthConfig {
                client_id: "  ".into(),
                client_secret: None,
            },
            1,
        );
        assert!(matches!(empty, Err(AuthError::NotConfigured)));
    }

    #[test]
    fn debug_output_redacts_secrets() {
        let config = GoogleOAuthConfig {
            client_id: "id".into(),
            client_secret: Some("super-secret".into()),
        };
        assert!(!format!("{config:?}").contains("super-secret"));
        let identity = GoogleIdentity {
            email: "a@b.c".into(),
            full_name: "A".into(),
            refresh_token: Some("1//refresh".into()),
        };
        assert!(
            !format!("{identity:?}").contains("1//refresh"),
            "{identity:?}"
        );
    }
}
