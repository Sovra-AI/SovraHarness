//! Built-in search sources: free, keyless, open services queried over HTTPS.
//!
//! Mwmbl is an open-source, non-profit web index; Wikipedia is queried through the
//! official `MediaWiki` API; a self-hosted `SearXNG` may be added by the user. Every
//! request names this app honestly and backs off when a service asks it to slow
//! down, so a busy source is skipped instead of hammered.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sovra_core::{SearchHit, parse_and_rank_searxng, parse_mwmbl, parse_wikipedia};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a source rests after it answers 429 or 503.
const COOLDOWN: Duration = Duration::from_secs(60);
/// Search API documents are small; anything larger is not a search response.
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MWMBL_SEARCH_URL: &str = "https://api.mwmbl.org/search/";
const WIKIPEDIA_SITE: &str = "en.wikipedia.org";
/// Encyclopedia articles complement web results; a few are enough.
const WIKIPEDIA_LIMIT: &str = "5";

/// Wikimedia's user-agent policy asks clients to identify themselves.
pub(super) const USER_AGENT: &str = concat!(
    "SovraHarness/",
    env!("CARGO_PKG_VERSION"),
    " (desktop assistant; https://github.com/Sovra-AI/SovraHarness)"
);

/// Which service a source talks to; also its name in `sources` and `errors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKind {
    Mwmbl,
    Wikipedia,
    SearXng,
}

impl SourceKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Mwmbl => "mwmbl",
            Self::Wikipedia => "wikipedia",
            Self::SearXng => "searxng",
        }
    }
}

/// One HTTP search source with its own back-off clock.
#[derive(Debug, Clone)]
pub(super) struct HttpSource {
    kind: SourceKind,
    client: reqwest::Client,
    /// Only used by [`SourceKind::SearXng`].
    base_url: String,
    resting_until: Arc<Mutex<Option<Instant>>>,
}

impl HttpSource {
    pub(super) fn new(kind: SourceKind, client: reqwest::Client, base_url: String) -> Self {
        Self {
            kind,
            client,
            base_url,
            resting_until: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) async fn search(&self, query: &str) -> Result<Vec<SearchHit>, String> {
        if self.is_resting() {
            return Err("resting after the service asked us to slow down".to_owned());
        }
        let request = match self.kind {
            SourceKind::Mwmbl => self.client.get(MWMBL_SEARCH_URL).query(&[("s", query)]),
            SourceKind::Wikipedia => self
                .client
                .get(format!("https://{WIKIPEDIA_SITE}/w/api.php"))
                .query(&[
                    ("action", "query"),
                    ("list", "search"),
                    ("srsearch", query),
                    ("srlimit", WIKIPEDIA_LIMIT),
                    ("srprop", "snippet"),
                    ("format", "json"),
                    ("formatversion", "2"),
                    ("utf8", "1"),
                ]),
            SourceKind::SearXng => self.client.get(searxng_search_url(&self.base_url, query)?),
        };
        let response = request
            .header(http::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(describe_request_error)?;
        let status = response.status();
        if status == http::StatusCode::TOO_MANY_REQUESTS
            || status == http::StatusCode::SERVICE_UNAVAILABLE
        {
            self.rest();
            return Err(format!("busy (HTTP {})", status.as_u16()));
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        let (bytes, _truncated) = read_capped(response, MAX_RESPONSE_BYTES).await?;
        let body = String::from_utf8_lossy(&bytes);
        match self.kind {
            SourceKind::Mwmbl => parse_mwmbl(&body),
            SourceKind::Wikipedia => parse_wikipedia(&body, WIKIPEDIA_SITE),
            SourceKind::SearXng => parse_and_rank_searxng(&body),
        }
    }

    fn is_resting(&self) -> bool {
        let mut guard = self
            .resting_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *guard {
            Some(until) if Instant::now() < until => true,
            Some(_) => {
                *guard = None;
                false
            }
            None => false,
        }
    }

    fn rest(&self) {
        tracing::info!(source = self.kind.name(), "search source busy; resting");
        let mut guard = self
            .resting_until
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(Instant::now() + COOLDOWN);
    }
}

/// Shared client for search APIs: honest user agent, short timeout, few redirects.
pub(super) fn api_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|error| error.to_string())
}

/// `GET {base}/search?q=&format=json` without credentials.
pub(crate) fn searxng_search_url(base: &str, query: &str) -> Result<reqwest::Url, String> {
    let base = base.trim().trim_end_matches('/');
    let mut url =
        reqwest::Url::parse(&format!("{base}/search")).map_err(|error| error.to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.clear();
        pairs.append_pair("q", query);
        pairs.append_pair("format", "json");
    }
    if url.query_pairs().any(|(key, _)| {
        let key = key.to_ascii_lowercase();
        key.contains("key") || key.contains("token") || key.contains("secret")
    }) {
        return Err("search URL must not carry an API key".to_owned());
    }
    Ok(url)
}

/// Validates a user-supplied `SearXNG` origin once at startup.
pub(super) fn normalize_searxng_base(base: &str) -> Result<String, String> {
    let base = base.trim().trim_end_matches('/').to_owned();
    let url = reqwest::Url::parse(&base).map_err(|error| error.to_string())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("SearXNG base URL must be http or https".to_owned());
    }
    if url.host_str().is_none() {
        return Err("SearXNG base URL needs a host".to_owned());
    }
    Ok(base)
}

/// Reads at most `max` bytes, so a hostile or broken server cannot exhaust memory.
/// Returns the bytes and whether the body was cut.
pub(super) async fn read_capped(
    mut response: reqwest::Response,
    max: usize,
) -> Result<(Vec<u8>, bool), String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(describe_request_error)? {
        let room = max.saturating_sub(bytes.len());
        if chunk.len() > room {
            bytes.extend_from_slice(&chunk[..room]);
            return Ok((bytes, true));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, false))
}

/// Short, secret-free error text for the model and the log.
pub(super) fn describe_request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out".to_owned()
    } else if error.is_connect() {
        "could not connect".to_owned()
    } else if error.is_redirect() {
        // The redirect policy's own reason (for example a blocked address) is the source.
        std::error::Error::source(&error)
            .map_or_else(|| "too many redirects".to_owned(), ToString::to_string)
    } else {
        error.without_url().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searxng_url_is_keyless_json() {
        let url = searxng_search_url("https://searx.example/", "rust async").expect("url");
        let url = url.as_str();
        assert!(url.starts_with("https://searx.example/search?"));
        assert!(url.contains("format=json"));
        assert!(url.contains("q=rust"));
        assert!(!url.to_ascii_lowercase().contains("api_key"));
        assert!(!url.to_ascii_lowercase().contains("token"));
    }

    #[test]
    fn searxng_base_must_be_an_http_origin() {
        assert_eq!(
            normalize_searxng_base(" https://searx.example/ ").as_deref(),
            Ok("https://searx.example")
        );
        assert!(normalize_searxng_base("ftp://searx.example").is_err());
        assert!(normalize_searxng_base("not a url").is_err());
    }

    #[test]
    fn user_agent_names_the_app() {
        assert!(USER_AGENT.starts_with("SovraHarness/"));
        assert!(!USER_AGENT.contains("Mozilla"));
    }

    #[test]
    fn a_busy_source_rests_then_recovers() {
        let source = HttpSource::new(SourceKind::Mwmbl, reqwest::Client::new(), String::new());
        assert!(!source.is_resting());
        source.rest();
        assert!(source.is_resting());
        *source.resting_until.lock().expect("test lock") = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(1))
                .expect("clock is past boot"),
        );
        assert!(!source.is_resting());
    }
}
