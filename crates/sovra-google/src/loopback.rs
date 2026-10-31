//! The browser half of sign-in (RFC 8252): a consent link with PKCE S256 and a
//! fresh `state`, and a single-use listener on `127.0.0.1` at a random port that
//! waits for Google to send the browser back.
//!
//! Blocking by design: the runtime runs [`GoogleSignIn::wait`] on a blocking
//! worker. Cancelling sets a flag and connects to the listener, which wakes the
//! blocked `accept` without polling or sleeping.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::Digest as _;
use sovra_core::google::{AuthorizationGrant, GoogleError, GoogleSignIn, SignInCanceller};

/// Browsers open speculative connections that never send a request; don't let
/// one of those hold up the real redirect.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST: usize = 16 * 1024;

pub(crate) struct PendingSignIn {
    url: String,
    listener: TcpListener,
    address: SocketAddr,
    state: String,
    verifier: String,
    redirect_uri: String,
    cancelled: Arc<AtomicBool>,
}

impl std::fmt::Debug for PendingSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingSignIn")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

fn random_token() -> Result<String, GoogleError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| GoogleError::Failed("Sovra couldn't start a safe sign-in.".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

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
        // Loopback only: never reachable from the network.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| {
            GoogleError::Failed("Sovra couldn't get ready for Google's reply.".into())
        })?;
        let address = listener.local_addr().map_err(|_| {
            GoogleError::Failed("Sovra couldn't get ready for Google's reply.".into())
        })?;
        let redirect_uri = format!("http://127.0.0.1:{}", address.port());
        let state = random_token()?;
        let verifier = random_token()?;
        let url = consent_url(
            client_id,
            &redirect_uri,
            scopes,
            &state,
            &code_challenge(&verifier),
            login_hint,
        );
        Ok(Self {
            url,
            listener,
            address,
            state,
            verifier,
            redirect_uri,
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl GoogleSignIn for PendingSignIn {
    fn url(&self) -> &str {
        &self.url
    }

    fn canceller(&self) -> SignInCanceller {
        let cancelled = self.cancelled.clone();
        let address = self.address;
        Arc::new(move || {
            cancelled.store(true, Ordering::SeqCst);
            // Wake the blocked accept; failure means it already returned.
            let _ = TcpStream::connect_timeout(&address, Duration::from_secs(1));
        })
    }

    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, GoogleError> {
        loop {
            let (mut stream, _) = self.listener.accept().map_err(|_| {
                GoogleError::Failed("Sovra stopped waiting for Google. Please try again.".into())
            })?;
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(GoogleError::Cancelled);
            }
            let Some(target) = read_request_target(&mut stream) else {
                continue;
            };
            match parse_callback(&target, &self.state) {
                Callback::Ignore => respond(&mut stream, "404 Not Found", ""),
                Callback::Code(code) => {
                    respond(&mut stream, "200 OK", SUCCESS_PAGE);
                    return Ok(AuthorizationGrant {
                        code,
                        verifier: self.verifier,
                        redirect_uri: self.redirect_uri,
                    });
                }
                Callback::Denied => {
                    respond(&mut stream, "200 OK", DENIED_PAGE);
                    return Err(GoogleError::NotAllowed(
                        "Google sign-in wasn't allowed, so nothing was connected.".into(),
                    ));
                }
            }
        }
    }
}

/// The request target of a `GET` (e.g. `/?code=…&state=…`), if one arrives in time.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    while !buffer.windows(2).any(|w| w == b"\r\n") && buffer.len() < MAX_REQUEST {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut parts = text.lines().next()?.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Callback {
    Code(String),
    Denied,
    /// Not Google's redirect (favicon, a stale tab, a forged `state`): keep waiting.
    Ignore,
}

pub(crate) fn parse_callback(target: &str, expected_state: &str) -> Callback {
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Ignore;
    };
    if url.path() != "/" {
        return Callback::Ignore;
    }
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if param("state").as_deref() != Some(expected_state) {
        return Callback::Ignore;
    }
    match (param("code"), param("error")) {
        (Some(code), None) if !code.is_empty() => Callback::Code(code),
        (_, Some(_)) => Callback::Denied,
        _ => Callback::Ignore,
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Self-contained pages: no scripts, fonts or remote assets.
const SUCCESS_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width\"><title>Sovra is connected</title>\
<style>body{font:1.25rem/1.6 system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1.5rem;\
color:#1f1633;background:#faf8ff}@media(prefers-color-scheme:dark){body{color:#f4f0ff;background:#15111f}}\
h1{font-size:2rem}</style></head><body><h1>&#10003; You're connected</h1>\
<p>Google is now connected to Sovra.</p><p>You can close this tab and go back to Sovra.</p></body></html>";

const DENIED_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width\"><title>Nothing was connected</title>\
<style>body{font:1.25rem/1.6 system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1.5rem;\
color:#1f1633;background:#faf8ff}@media(prefers-color-scheme:dark){body{color:#f4f0ff;background:#15111f}}\
h1{font-size:2rem}</style></head><body><h1>Nothing was connected</h1>\
<p>That's fine. You can close this tab. To try again, press <b>Connect Google</b> in Sovra.</p>\
</body></html>";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        // RFC 7636 appendix B.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

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

    #[test]
    fn only_googles_redirect_with_our_state_counts() {
        assert_eq!(
            parse_callback("/?state=s1&code=4%2Fabc", "s1"),
            Callback::Code("4/abc".into())
        );
        assert_eq!(
            parse_callback("/?state=s1&error=access_denied", "s1"),
            Callback::Denied
        );
        assert_eq!(
            parse_callback("/?state=forged&code=x", "s1"),
            Callback::Ignore
        );
        assert_eq!(parse_callback("/favicon.ico", "s1"), Callback::Ignore);
        assert_eq!(parse_callback("/?state=s1", "s1"), Callback::Ignore);
    }

    #[test]
    fn browser_redirect_completes_and_cancel_wakes_the_listener() {
        let pending = PendingSignIn::open("id", &["openid"], None).expect("bind loopback");
        let url = url::Url::parse(pending.url()).expect("valid URL");
        let state = url
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .expect("state");
        let address = pending.address;
        // A speculative connection that sends nothing, then the real redirect.
        let idle = TcpStream::connect(address).expect("connect");
        drop(idle);
        let mut browser = TcpStream::connect(address).expect("connect");
        write!(
            browser,
            "GET /?code=abc&state={state} HTTP/1.1\r\nHost: x\r\n\r\n"
        )
        .expect("send");
        let grant = Box::new(pending).wait().expect("grant");
        assert_eq!(grant.code, "abc");
        assert!(grant.redirect_uri.starts_with("http://127.0.0.1:"));
        let mut page = String::new();
        let _ = browser.read_to_string(&mut page);
        assert!(page.contains("You're connected"));

        let pending = PendingSignIn::open("id", &["openid"], None).expect("bind loopback");
        pending.canceller()();
        assert_eq!(Box::new(pending).wait(), Err(GoogleError::Cancelled));
    }
}
