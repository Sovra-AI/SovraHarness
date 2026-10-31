//! Sign-in accounts and sessions for the single-binary login system.
//!
//! Pure domain only: email/password validation, the RFC 8252 loopback callback
//! contract for Google Sign-In (parsing and constant-time checks, no sockets),
//! the `AuthCommand`/`AuthEvent` protocol and the reducer the UI folds.
//! Argon2id hashing and the Google token exchange live in `sovra-providers`;
//! the loopback listener and persistence live in `sovra-runtime`.

use serde::{Deserialize, Serialize};

use crate::memory::Timestamp;

/// Below this, a password is rejected outright (NIST SP 800-63B minimum).
pub const MIN_PASSWORD_CHARS: usize = 8;

/// How a person proves who they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthProvider {
    /// Email + password, hashed with Argon2id and checked locally.
    Password,
    /// Google Sign-In (OAuth 2.0 authorization code + PKCE, loopback redirect).
    Google,
}

impl AuthProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Password => "Email & password",
            Self::Google => "Google",
        }
    }
}

/// The signed-in person, as shown in the UI. Never carries hashes or tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSession {
    pub account_id: String,
    pub email: String,
    pub full_name: String,
    pub provider: AuthProvider,
    pub signed_in_at: Timestamp,
}

impl AuthSession {
    /// Up to two initials for the avatar chip, from the first and last word.
    pub fn initials(&self) -> String {
        initials(&self.full_name, &self.email)
    }
}

/// A stored account. The Argon2id PHC string never leaves the runtime and store;
/// events toward the UI carry [`AuthSession`] instead.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthAccount {
    pub id: String,
    /// Normalized (trimmed, lowercased) — the unique key for sign-in.
    pub email: String,
    pub full_name: String,
    pub provider: AuthProvider,
    /// PHC-format Argon2id hash for password accounts; `None` for Google accounts.
    pub password_phc: Option<String>,
    pub created_at: Timestamp,
}

impl std::fmt::Debug for AuthAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The PHC string is not reversible, but it still never belongs in logs.
        f.debug_struct("AuthAccount")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("full_name", &self.full_name)
            .field("provider", &self.provider)
            .field(
                "password_phc",
                &self.password_phc.as_ref().map(|_| "⟨redacted⟩"),
            )
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Everything the store persists for sign-in: who exists, and who is signed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthRecords {
    pub accounts: Vec<AuthAccount>,
    pub session: Option<AuthSession>,
}

impl AuthRecords {
    pub fn account_by_email(&self, normalized_email: &str) -> Option<&AuthAccount> {
        self.accounts.iter().find(|a| a.email == normalized_email)
    }
}

/// Trims, lowercases and sanity-checks an email address. Deliberately lenient
/// beyond the obvious mistakes: the address book is local, not a mail server.
pub fn normalize_email(input: &str) -> Result<String, String> {
    let email = input.trim().to_lowercase();
    if email.is_empty() {
        return Err("Enter your email address.".into());
    }
    let mut parts = email.split('@');
    let (local, domain) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let valid = parts.next().is_none()
        && !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.chars().any(char::is_whitespace);
    if valid {
        Ok(email)
    } else {
        Err("That doesn't look like an email address. Check for typos, like a missing @.".into())
    }
}

/// How strong a password is, shown live while typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PasswordStrength {
    TooShort,
    Weak,
    Good,
    Strong,
}

impl PasswordStrength {
    pub fn label(self) -> &'static str {
        match self {
            Self::TooShort => "Too short — use at least 8 characters",
            Self::Weak => "Weak — add more words or symbols",
            Self::Good => "Good password",
            Self::Strong => "Strong password",
        }
    }

    /// Whether account creation accepts this password.
    pub fn accepted(self) -> bool {
        self >= Self::Weak
    }
}

/// Length-first scoring (NIST SP 800-63B): long passphrases beat short noise.
pub fn password_strength(password: &str) -> PasswordStrength {
    let chars = password.chars().count();
    if chars < MIN_PASSWORD_CHARS {
        return PasswordStrength::TooShort;
    }
    let classes = [
        password.chars().any(|c| c.is_ascii_lowercase()),
        password.chars().any(|c| c.is_ascii_uppercase()),
        password.chars().any(|c| c.is_ascii_digit()),
        password.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|present| **present)
    .count();
    if chars >= 16 || (chars >= 12 && classes >= 3) {
        PasswordStrength::Strong
    } else if chars >= 10 || classes >= 3 {
        PasswordStrength::Good
    } else {
        PasswordStrength::Weak
    }
}

/// Up to two initials from a name, falling back to the email's first letter.
pub fn initials(full_name: &str, email: &str) -> String {
    let mut words = full_name.split_whitespace();
    let first = words.next();
    let last = words.next_back();
    let letters: String = [first, last]
        .into_iter()
        .flatten()
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    if letters.is_empty() {
        email
            .chars()
            .next()
            .map_or_else(|| "?".to_owned(), |c| c.to_uppercase().collect())
    } else {
        letters
    }
}

/// What the person's browser sent back to the loopback listener (RFC 8252 §7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OauthCallback {
    Granted { code: String, state: String },
    Denied { reason: String },
}

/// Parses the request line of the browser's `GET` against the loopback listener,
/// e.g. `GET /callback?code=4%2F0Af…&state=xyz HTTP/1.1`. Returns `None` for
/// anything that is not the expected callback (favicons, probes, other paths).
pub fn parse_oauth_redirect(request_line: &str) -> Option<OauthCallback> {
    let target = request_line.strip_prefix("GET ")?.split(' ').next()?;
    let query = target.strip_prefix("/callback?")?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "code" => code = Some(percent_decode(value)),
            "state" => state = Some(percent_decode(value)),
            "error" => error = Some(percent_decode(value)),
            _ => {}
        }
    }
    if let Some(reason) = error {
        return Some(OauthCallback::Denied { reason });
    }
    match (code, state) {
        (Some(code), Some(state)) if !code.is_empty() && !state.is_empty() => {
            Some(OauthCallback::Granted { code, state })
        }
        _ => Some(OauthCallback::Denied {
            reason: "invalid_response".to_owned(),
        }),
    }
}

/// Percent-decodes a query value (`+` as space). Invalid escapes pass through.
pub fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| {
                    std::str::from_utf8(h)
                        .ok()
                        .and_then(|h| u8::from_str_radix(h, 16).ok())
                });
                if let Some(byte) = hex {
                    out.push(byte);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            byte => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Compares the returned `state` with the expected one without leaking length
/// or position through timing (RFC 8252 §8.9 CSRF protection).
pub fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

/// A request from the UI about signing in. All fields arrive raw; the runtime
/// validates and never echoes passwords back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCommand {
    /// Load persisted accounts and restore a saved session.
    Load,
    CreateAccount {
        email: String,
        password: String,
        full_name: String,
    },
    SignIn {
        email: String,
        password: String,
    },
    /// Open the Google consent page in the browser and await the loopback redirect.
    StartGoogleSignIn,
    /// Abandon a waiting Google flow (closes the loopback listener).
    CancelGoogleSignIn,
    SignOut,
}

/// Where a sign-in problem belongs, so the form can point at the right field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Email,
    Password,
    FullName,
    General,
}

/// A notification from the runtime about sign-in state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
    /// Startup answer to [`AuthCommand::Load`].
    Loaded {
        session: Option<AuthSession>,
        has_accounts: bool,
        /// Whether Google Sign-In is configured (a client ID exists).
        google_ready: bool,
    },
    /// Work started; the form shows progress and disables double submits.
    Submitting,
    SignedIn {
        session: AuthSession,
        new_account: bool,
    },
    /// The consent page is ready; the UI opens `auth_url` in the browser and
    /// waits for the loopback redirect.
    GoogleSignInStarted {
        auth_url: String,
    },
    GoogleSignInCancelled,
    SignedOut,
    Failed {
        field: AuthField,
        message: String,
    },
}

/// Where the sign-in flow stands, folded by [`AuthState::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthPhase {
    /// Nothing reported yet (startup, or tests without an auth runtime).
    #[default]
    Unknown,
    SignedOut,
    Submitting,
    /// Browser open, loopback listener waiting for Google's redirect. The URL
    /// stays available so people can copy it if the browser failed to open.
    WaitingForGoogle {
        auth_url: String,
    },
    SignedIn(AuthSession),
}

/// Sign-in state as the UI sees it. Pure; fold events with [`AuthState::apply`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthState {
    pub phase: AuthPhase,
    pub error: Option<(AuthField, String)>,
    pub has_accounts: bool,
    pub google_ready: bool,
}

impl AuthState {
    pub fn apply(&mut self, event: &AuthEvent) {
        match event {
            AuthEvent::Loaded {
                session,
                has_accounts,
                google_ready,
            } => {
                self.has_accounts = *has_accounts;
                self.google_ready = *google_ready;
                self.error = None;
                self.phase = match session {
                    Some(session) => AuthPhase::SignedIn(session.clone()),
                    None => AuthPhase::SignedOut,
                };
            }
            AuthEvent::Submitting => {
                self.error = None;
                self.phase = AuthPhase::Submitting;
            }
            AuthEvent::SignedIn { session, .. } => {
                self.has_accounts = true;
                self.error = None;
                self.phase = AuthPhase::SignedIn(session.clone());
            }
            AuthEvent::GoogleSignInStarted { auth_url } => {
                self.error = None;
                self.phase = AuthPhase::WaitingForGoogle {
                    auth_url: auth_url.clone(),
                };
            }
            AuthEvent::GoogleSignInCancelled => {
                if matches!(self.phase, AuthPhase::WaitingForGoogle { .. }) {
                    self.phase = AuthPhase::SignedOut;
                }
            }
            AuthEvent::SignedOut => {
                self.error = None;
                self.phase = AuthPhase::SignedOut;
            }
            AuthEvent::Failed { field, message } => {
                if !matches!(self.phase, AuthPhase::SignedIn(_)) {
                    self.phase = AuthPhase::SignedOut;
                }
                self.error = Some((*field, message.clone()));
            }
        }
    }

    pub fn session(&self) -> Option<&AuthSession> {
        match &self.phase {
            AuthPhase::SignedIn(session) => Some(session),
            AuthPhase::Unknown
            | AuthPhase::SignedOut
            | AuthPhase::Submitting
            | AuthPhase::WaitingForGoogle { .. } => None,
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.session().is_some()
    }

    /// True while a submit or the Google hand-off is in flight.
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            AuthPhase::Submitting | AuthPhase::WaitingForGoogle { .. }
        )
    }
}

/// The page the browser shows after the loopback redirect, so people know to
/// come back to the app. Plain inline HTML: no assets, no scripts.
pub fn callback_page(success: bool) -> String {
    let (title, detail) = if success {
        (
            "You're signed in",
            "Sovra is ready. You can close this tab and return to the app.",
        )
    } else {
        (
            "Sign-in didn't finish",
            "You can close this tab and try again from the Sovra app.",
        )
    };
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
         <!doctype html><html><head><meta charset=\"utf-8\"><title>{title} · Sovra</title></head>\
         <body style=\"font-family:system-ui,sans-serif;display:flex;align-items:center;\
         justify-content:center;height:100vh;margin:0;background:#101014;color:#f4f4f6\">\
         <div style=\"text-align:center;max-width:26rem;padding:2rem\">\
         <h1 style=\"font-size:1.4rem;margin-bottom:.5rem\">{title}</h1>\
         <p style=\"opacity:.75;line-height:1.5\">{detail}</p></div></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emails_normalize_and_reject_typos() {
        assert_eq!(
            normalize_email("  Barney@Sovra.AI "),
            Ok("barney@sovra.ai".to_owned())
        );
        for bad in [
            "",
            "barney",
            "barney@",
            "@sovra.ai",
            "a@b",
            "a b@c.d",
            "a@@b.c",
        ] {
            assert!(normalize_email(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn password_strength_prefers_length() {
        assert_eq!(password_strength("short"), PasswordStrength::TooShort);
        assert_eq!(password_strength("aaaaaaaa"), PasswordStrength::Weak);
        assert!(password_strength("aaaaaaaa").accepted());
        assert_eq!(password_strength("aaaaaaaaaa"), PasswordStrength::Good);
        assert_eq!(password_strength("Str0ng-pass!"), PasswordStrength::Strong);
        assert_eq!(
            password_strength("correct horse battery staple"),
            PasswordStrength::Strong
        );
    }

    #[test]
    fn initials_use_first_and_last_word_or_email() {
        assert_eq!(initials("Barney the Jedi", "x@y.z"), "BJ");
        assert_eq!(initials("Barney", "x@y.z"), "B");
        assert_eq!(initials("", "barney@sovra.ai"), "B");
        assert_eq!(initials("", ""), "?");
    }

    #[test]
    fn oauth_redirect_parses_grants_denials_and_noise() {
        assert_eq!(
            parse_oauth_redirect("GET /callback?state=abc&code=4%2F0AfX HTTP/1.1"),
            Some(OauthCallback::Granted {
                code: "4/0AfX".to_owned(),
                state: "abc".to_owned(),
            })
        );
        assert_eq!(
            parse_oauth_redirect("GET /callback?error=access_denied&state=abc HTTP/1.1"),
            Some(OauthCallback::Denied {
                reason: "access_denied".to_owned(),
            })
        );
        assert_eq!(
            parse_oauth_redirect("GET /callback?code=&state= HTTP/1.1"),
            Some(OauthCallback::Denied {
                reason: "invalid_response".to_owned(),
            })
        );
        assert_eq!(parse_oauth_redirect("GET /favicon.ico HTTP/1.1"), None);
        assert_eq!(parse_oauth_redirect("POST /callback?code=x HTTP/1.1"), None);
    }

    #[test]
    fn percent_decoding_handles_escapes_plus_and_junk() {
        assert_eq!(percent_decode("a%2Fb+c%20d"), "a/b c d");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn state_comparison_is_exact() {
        assert!(constant_time_str_eq("abc123", "abc123"));
        assert!(!constant_time_str_eq("abc123", "abc124"));
        assert!(!constant_time_str_eq("abc", "abc123"));
        assert!(!constant_time_str_eq("", "a"));
        assert!(constant_time_str_eq("", ""));
    }

    fn session() -> AuthSession {
        AuthSession {
            account_id: "acct-1".into(),
            email: "barney@sovra.ai".into(),
            full_name: "Barney the Jedi".into(),
            provider: AuthProvider::Password,
            signed_in_at: Timestamp(1),
        }
    }

    #[test]
    fn reducer_walks_the_whole_flow() {
        let mut state = AuthState::default();
        assert_eq!(state.phase, AuthPhase::Unknown);

        state.apply(&AuthEvent::Loaded {
            session: None,
            has_accounts: false,
            google_ready: true,
        });
        assert_eq!(state.phase, AuthPhase::SignedOut);
        assert!(state.google_ready && !state.has_accounts);

        state.apply(&AuthEvent::Submitting);
        assert!(state.busy());
        state.apply(&AuthEvent::Failed {
            field: AuthField::Password,
            message: "Wrong password.".into(),
        });
        assert_eq!(state.phase, AuthPhase::SignedOut);
        assert_eq!(
            state.error,
            Some((AuthField::Password, "Wrong password.".to_owned()))
        );

        state.apply(&AuthEvent::GoogleSignInStarted {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth?x=1".into(),
        });
        assert!(state.busy() && state.error.is_none());
        assert!(
            matches!(&state.phase, AuthPhase::WaitingForGoogle { auth_url } if auth_url.contains("accounts.google.com"))
        );
        state.apply(&AuthEvent::GoogleSignInCancelled);
        assert_eq!(state.phase, AuthPhase::SignedOut);

        state.apply(&AuthEvent::SignedIn {
            session: session(),
            new_account: true,
        });
        assert!(state.is_signed_in() && state.has_accounts);

        state.apply(&AuthEvent::SignedOut);
        assert_eq!(state.phase, AuthPhase::SignedOut);
    }

    #[test]
    fn loaded_session_restores_sign_in_and_debug_redacts_hashes() {
        let mut state = AuthState::default();
        state.apply(&AuthEvent::Loaded {
            session: Some(session()),
            has_accounts: true,
            google_ready: false,
        });
        assert!(state.is_signed_in());

        let account = AuthAccount {
            id: "acct-1".into(),
            email: "barney@sovra.ai".into(),
            full_name: "Barney".into(),
            provider: AuthProvider::Password,
            password_phc: Some("$argon2id$v=19$secret".into()),
            created_at: Timestamp(1),
        };
        let debug = format!("{account:?}");
        assert!(
            !debug.contains("argon2id"),
            "hash leaked into Debug: {debug}"
        );
    }
}
