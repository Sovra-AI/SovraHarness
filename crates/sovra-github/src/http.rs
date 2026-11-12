//! Bounded GitHub HTTP: every body is size-capped, server text never reaches people.

use futures::StreamExt as _;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::Value;
use sovra_core::github::{GitHubError, SIGN_IN_AGAIN};

/// Repository file contents arrive base64 in JSON; files GitHub serves this way
/// are at most 1 MB, so 8 MB leaves room without letting a reply grow unbounded.
const JSON_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum CallError {
    /// 401: the access was rejected.
    Unauthorized,
    /// 404: the repository, issue or file isn't there, or is private to others.
    NotFound,
    /// 429, or a 403 with no requests left.
    Busy,
    /// 403 for any other reason.
    NotAllowed,
    /// Anything else, already in plain words.
    Failed(String),
}

impl CallError {
    pub(crate) fn into_github(self) -> GitHubError {
        match self {
            Self::Unauthorized => GitHubError::SignInNeeded(SIGN_IN_AGAIN.into()),
            Self::NotFound => GitHubError::Failed(
                "GitHub couldn't find that. Check the name; a private repository also needs \
                 the person's access."
                    .into(),
            ),
            Self::Busy => GitHubError::Failed(
                "GitHub is busy or has had a lot of requests from Sovra. Try again in a few \
                 minutes."
                    .into(),
            ),
            Self::NotAllowed => GitHubError::Failed(
                "GitHub didn't allow that. The repository may belong to an organisation that \
                 hasn't approved Sovra yet."
                    .into(),
            ),
            Self::Failed(message) => GitHubError::Failed(message),
        }
    }
}

/// The headers every API call carries.
pub(crate) fn api_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    if let Ok(mut value) = HeaderValue::from_str(&format!("Bearer {token}")) {
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    headers
}

fn network_error(_: &reqwest::Error) -> CallError {
    CallError::Failed("Sovra couldn't reach GitHub. Check your internet connection.".into())
}

/// Sends `request` and reads a successful JSON body of at most [`JSON_LIMIT`] bytes.
pub(crate) async fn read_json(request: reqwest::RequestBuilder) -> Result<Value, CallError> {
    let response = request.send().await.map_err(|e| network_error(&e))?;
    let status = response.status().as_u16();
    let out_of_requests = response
        .headers()
        .get("x-ratelimit-remaining")
        .is_some_and(|value| value.as_bytes() == b"0");
    if !response.status().is_success() {
        return Err(classify(status, out_of_requests));
    }
    if response
        .content_length()
        .is_some_and(|n| n > JSON_LIMIT as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| network_error(&e))?;
        if bytes.len().saturating_add(chunk.len()) > JSON_LIMIT {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CallError::Failed("GitHub sent an answer Sovra couldn't read.".into()))
}

fn too_large() -> CallError {
    CallError::Failed("That is too large for Sovra to read in one go.".into())
}

pub(crate) fn classify(status: u16, out_of_requests: bool) -> CallError {
    match status {
        401 => CallError::Unauthorized,
        404 => CallError::NotFound,
        429 => CallError::Busy,
        403 if out_of_requests => CallError::Busy,
        403 => CallError::NotAllowed,
        410 => CallError::Failed("That repository has this feature switched off.".into()),
        422 => CallError::Failed(
            "GitHub couldn't understand that request. Try asking in a different way.".into(),
        ),
        _ => CallError::Failed("GitHub had a problem. Try again in a minute.".into()),
    }
}
