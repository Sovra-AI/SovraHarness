//! Bounded Google HTTP: every body is size-capped, server text never reaches people.

use futures::StreamExt as _;
use serde_json::Value;
use sovra_core::google::{GoogleError, GoogleRequest, service_info};

/// JSON from Google APIs is small; file contents use their own larger cap.
pub(crate) const JSON_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const FILE_LIMIT: usize = 10 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum CallError {
    /// 401: the access token was rejected.
    Unauthorized,
    /// 403 `insufficientPermissions`: the person did not allow this service.
    NotAllowed,
    /// 404: the id the model passed does not exist.
    NotFound,
    /// 429 or a rate-limit 403.
    Busy,
    /// Anything else, already in plain words.
    Failed(String),
    /// The refresh itself failed (sign-in revoked or expired).
    SignIn(GoogleError),
}

impl CallError {
    pub(crate) fn into_google(self, request: &GoogleRequest) -> GoogleError {
        let product = service_info(request.service()).product;
        match self {
            Self::Unauthorized => GoogleError::SignInNeeded(
                "Google asked Sovra to sign in again. Open Connections and press Sign in again."
                    .into(),
            ),
            Self::NotAllowed => GoogleError::NotAllowed(format!(
                "Google hasn't allowed Sovra to use {product}. Turn it on in Connections."
            )),
            Self::NotFound => GoogleError::Failed(format!("{product} couldn't find that item.")),
            Self::Busy => GoogleError::Failed(format!(
                "{product} is busy right now. Try again in a minute."
            )),
            Self::Failed(message) => GoogleError::Failed(message),
            Self::SignIn(error) => error,
        }
    }
}

impl From<GoogleError> for CallError {
    fn from(error: GoogleError) -> Self {
        Self::SignIn(error)
    }
}

pub(crate) fn network_error(_: &reqwest::Error) -> CallError {
    CallError::Failed("Sovra couldn't reach Google. Check your internet connection.".into())
}

/// Sends `request` and reads at most `limit` bytes of a successful body.
pub(crate) async fn read_bytes(
    request: reqwest::RequestBuilder,
    limit: usize,
) -> Result<(Vec<u8>, Option<String>), CallError> {
    let response = request.send().await.map_err(|e| network_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        let body = collect(response, 64 * 1024).await.unwrap_or_default();
        return Err(classify(status.as_u16(), &body));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(too_large());
    }
    Ok((collect(response, limit).await?, content_type))
}

pub(crate) async fn read_json(request: reqwest::RequestBuilder) -> Result<Value, CallError> {
    let (bytes, _) = read_bytes(request, JSON_LIMIT).await?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| CallError::Failed("Google sent an answer Sovra couldn't read.".into()))
}

async fn collect(response: reqwest::Response, limit: usize) -> Result<Vec<u8>, CallError> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| network_error(&e))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn too_large() -> CallError {
    CallError::Failed("That is too large for Sovra to read in one go.".into())
}

/// Google's error body: `{"error":{"code","status","errors":[{"reason"}]}}`.
pub(crate) fn classify(status: u16, body: &[u8]) -> CallError {
    let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let error = &parsed["error"];
    let reason = error["errors"][0]["reason"]
        .as_str()
        .or_else(|| error["status"].as_str())
        .unwrap_or_default();
    match status {
        401 => CallError::Unauthorized,
        404 => CallError::NotFound,
        429 => CallError::Busy,
        403 if reason.contains("RateLimit") || reason.contains("rateLimit") => CallError::Busy,
        403 if reason == "insufficientPermissions"
            || reason == "PERMISSION_DENIED"
            || reason == "ACCESS_TOKEN_SCOPE_INSUFFICIENT" =>
        {
            CallError::NotAllowed
        }
        403 => CallError::Failed("Google didn't allow that.".into()),
        400 => CallError::Failed(
            "Google couldn't understand that request. Try asking in a different way.".into(),
        ),
        _ => CallError::Failed("Google had a problem. Try again in a minute.".into()),
    }
}
