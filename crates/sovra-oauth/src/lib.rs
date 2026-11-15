//! The browser half of sign-in (RFC 8252): a consent link with PKCE S256 and a
//! fresh `state`, and a single-use listener on `127.0.0.1` at a random port that
//! waits for the provider to send the browser back.
//!
//! Blocking by design: the runtime runs [`NativeSignIn::wait`] on a blocking
//! worker. Cancelling sets a flag and connects to the listener, which wakes the
//! blocked `accept` without polling or sleeping.

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::Digest as _;
use sovra_core::oauth::{AuthorizationGrant, NativeSignIn, OAuthError, SignInCanceller};

/// Microsoft ignores ephemeral ports only for `localhost` redirect URIs.
/// Both variants bind the actual listener exclusively to IPv4 loopback.
#[derive(Debug, Clone, Copy)]
pub enum RedirectHost {
    IpV4,
    Localhost,
}

impl RedirectHost {
    fn name(self) -> &'static str {
        match self {
            Self::IpV4 => "127.0.0.1",
            Self::Localhost => "localhost",
        }
    }
}

/// Browsers open speculative connections that never send a request; don't let
/// one of those hold up the real redirect.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REQUEST: usize = 16 * 1024;

pub struct LoopbackSignIn {
    url: String,
    listener: TcpListener,
    address: SocketAddr,
    state: String,
    verifier: String,
    redirect_uri: String,
    cancelled: Arc<AtomicBool>,
}

impl std::fmt::Debug for LoopbackSignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopbackSignIn")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

fn random_token() -> Result<String, OAuthError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| OAuthError::Failed("Sovra couldn't start a safe sign-in.".into()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

impl LoopbackSignIn {
    pub fn open(
        host: RedirectHost,
        build_url: impl FnOnce(&str, &str, &str) -> String,
    ) -> Result<Self, OAuthError> {
        // Loopback only: never reachable from the network.
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| {
            OAuthError::Failed("Sovra couldn't get ready for the browser's reply.".into())
        })?;
        let address = listener.local_addr().map_err(|_| {
            OAuthError::Failed("Sovra couldn't get ready for the browser's reply.".into())
        })?;
        let redirect_uri = format!("http://{}:{}", host.name(), address.port());
        let state = random_token()?;
        let verifier = random_token()?;
        let url = build_url(&redirect_uri, &state, &code_challenge(&verifier));
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

impl NativeSignIn for LoopbackSignIn {
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

    fn wait(self: Box<Self>) -> Result<AuthorizationGrant, OAuthError> {
        loop {
            let (mut stream, _) = self.listener.accept().map_err(|_| {
                OAuthError::Failed(
                    "Sovra stopped waiting for your browser. Please try again.".into(),
                )
            })?;
            if self.cancelled.load(Ordering::SeqCst) {
                return Err(OAuthError::Cancelled);
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
                    return Err(OAuthError::Denied);
                }
            }
        }
    }
}

/// The request target of a `GET` (e.g. `/?code=…&state=…`), if one arrives in time.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok()?;
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    while !buffer.windows(2).any(|w| w == b"\r\n") && buffer.len() < MAX_REQUEST {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        stream.set_read_timeout(Some(remaining)).ok()?;
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    if buffer.len() >= MAX_REQUEST {
        return None;
    }
    let text = String::from_utf8_lossy(&buffer);
    let mut parts = text.lines().next()?.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_owned))?
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Callback {
    Code(String),
    Denied,
    /// Not the sign-in provider's redirect (favicon, a stale tab, a forged `state`): keep waiting.
    Ignore,
}

pub(crate) fn parse_callback(target: &str, expected_state: &str) -> Callback {
    if !target.starts_with("/?") {
        return Callback::Ignore;
    }
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Callback::Ignore;
    };
    if url.path() != "/" {
        return Callback::Ignore;
    }
    for name in ["state", "code", "error"] {
        if url.query_pairs().filter(|(key, _)| key == name).count() > 1 {
            return Callback::Ignore;
        }
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
    let _ = stream.set_write_timeout(Some(READ_TIMEOUT));
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Self-contained pages: no scripts, fonts or remote assets.
const SUCCESS_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width\"><title>Return to Sovra</title>\
<style>body{font:1.25rem/1.6 system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1.5rem;\
color:#1f1633;background:#faf8ff}@media(prefers-color-scheme:dark){body{color:#f4f0ff;background:#15111f}}\
h1{font-size:2rem}</style></head><body><h1>&#10003; Return to Sovra</h1>\
<p>Sovra is finishing your sign-in. Check the app for confirmation.</p><p>You can close this tab and go back to Sovra.</p></body></html>";

const DENIED_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width\"><title>Nothing was connected</title>\
<style>body{font:1.25rem/1.6 system-ui,sans-serif;max-width:32rem;margin:15vh auto;padding:0 1.5rem;\
color:#1f1633;background:#faf8ff}@media(prefers-color-scheme:dark){body{color:#f4f0ff;background:#15111f}}\
h1{font-size:2rem}</style></head><body><h1>Nothing was connected</h1>\
<p>That's fine. You can close this tab. To try again, press <b>Connect</b> in Sovra.</p>\
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
    fn only_the_redirect_with_our_state_counts() {
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
        for target in [
            "/?state=s1&state=s1&code=x",
            "/?state=s1&code=x&code=y",
            "/?state=s1&error=denied&error=denied",
            "//attacker.test/?state=s1&code=x",
            "http://127.0.0.1/?state=s1&code=x",
            "/callback?state=s1&code=x",
        ] {
            assert_eq!(parse_callback(target, "s1"), Callback::Ignore);
        }
    }

    #[test]
    fn browser_redirect_completes_and_cancel_wakes_the_listener() {
        let pending = LoopbackSignIn::open(RedirectHost::IpV4, |redirect, state, challenge| {
            format!("https://example.test/authorize?redirect_uri={redirect}&state={state}&code_challenge={challenge}")
        }).expect("bind loopback");
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
        assert!(page.contains("Return to Sovra"));

        let pending = LoopbackSignIn::open(RedirectHost::IpV4, |redirect, state, challenge| {
            format!("https://example.test/authorize?redirect_uri={redirect}&state={state}&code_challenge={challenge}")
        }).expect("bind loopback");
        pending.canceller()();
        assert_eq!(Box::new(pending).wait(), Err(OAuthError::Cancelled));
    }
}
